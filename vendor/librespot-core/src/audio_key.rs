use std::{collections::HashMap, io::Write, time::Duration};

use byteorder::{BigEndian, ByteOrder, WriteBytesExt};
use bytes::Bytes;
use thiserror::Error;
use tokio::sync::oneshot;

use crate::{Error, FileId, SpotifyId, packet::PacketType, util::SeqGenerator};

#[derive(Debug, Hash, PartialEq, Eq, Copy, Clone)]
pub struct AudioKey(pub [u8; 16]);

#[derive(Debug, Error)]
pub enum AudioKeyError {
    // SpotiLite: the error code sent by Spotify is kept (0x0001 permanent denial,
    // 0x0002 transient denial) so that transient denials can be retried.
    #[error("audio key error {0:#06x}")]
    AesKey(u16),
    #[error("other end of channel disconnected")]
    Channel,
    #[error("unexpected packet type {0}")]
    Packet(u8),
    #[error("sequence {0} not pending")]
    Sequence(u32),
    #[error("audio key response timeout")]
    Timeout,
}

impl From<AudioKeyError> for Error {
    fn from(err: AudioKeyError) -> Self {
        match err {
            AudioKeyError::AesKey(_) => Error::unavailable(err),
            AudioKeyError::Channel => Error::aborted(err),
            AudioKeyError::Sequence(_) => Error::aborted(err),
            AudioKeyError::Packet(_) => Error::unimplemented(err),
            AudioKeyError::Timeout => Error::aborted(err),
        }
    }
}

/// Denial code Spotify sends when a key request should simply be tried again.
const AUDIO_KEY_ERROR_TRANSIENT: u16 = 0x0002;
/// Attempts per key (SpotiLite, after librespot-org/librespot#1763).
const KEY_REQUEST_ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(1);
/// 1.5 s upstream; a little more headroom right after an access point reconnect.
const KEY_RESPONSE_TIMEOUT: Duration = Duration::from_millis(2500);

component! {
    AudioKeyManager : AudioKeyManagerInner {
        sequence: SeqGenerator<u32> = SeqGenerator::new(0),
        pending: HashMap<u32, oneshot::Sender<Result<AudioKey, Error>>> = HashMap::new(),
    }
}

/// Removes a request from the pending map however it ends (answer, timeout or
/// cancellation), so timed-out requests do not accumulate.
struct PendingGuard<'a> {
    manager: &'a AudioKeyManager,
    seq: u32,
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        let seq = self.seq;
        self.manager.lock(|inner| {
            inner.pending.remove(&seq);
        });
    }
}

impl AudioKeyManager {
    pub(crate) fn dispatch(&self, cmd: PacketType, mut data: Bytes) -> Result<(), Error> {
        let seq = BigEndian::read_u32(data.split_to(4).as_ref());

        let sender = self
            .lock(|inner| inner.pending.remove(&seq))
            .ok_or(AudioKeyError::Sequence(seq))?;

        match cmd {
            PacketType::AesKey => {
                let mut key = [0u8; 16];
                key.copy_from_slice(data.as_ref());
                sender
                    .send(Ok(AudioKey(key)))
                    .map_err(|_| AudioKeyError::Channel)?
            }
            PacketType::AesKeyError => {
                let code = if data.len() >= 2 {
                    BigEndian::read_u16(&data[..2])
                } else {
                    0
                };
                error!("error audio key {:x} {:x}", code >> 8, code & 0xff);
                sender
                    .send(Err(AudioKeyError::AesKey(code).into()))
                    .map_err(|_| AudioKeyError::Channel)?
            }
            _ => {
                trace!("Did not expect {cmd:?} AES key packet with data {data:#?}");
                return Err(AudioKeyError::Packet(cmd as u8).into());
            }
        }

        Ok(())
    }

    pub async fn request(&self, track: SpotifyId, file: FileId) -> Result<AudioKey, Error> {
        let mut attempt = 1;
        loop {
            match self.request_once(track, file).await {
                Ok(key) => return Ok(key),
                Err(RequestError { retryable, error }) => {
                    if !retryable || attempt >= KEY_REQUEST_ATTEMPTS {
                        return Err(error);
                    }
                    warn!(
                        "Audio key request failed ({error}), retrying ({attempt}/{})",
                        KEY_REQUEST_ATTEMPTS - 1
                    );
                    tokio::time::sleep(RETRY_DELAY * attempt).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn request_once(&self, track: SpotifyId, file: FileId) -> Result<AudioKey, RequestError> {
        let (tx, rx) = oneshot::channel();

        let seq = self.lock(move |inner| {
            let seq = inner.sequence.get();
            inner.pending.insert(seq, tx);
            seq
        });
        let _guard = PendingGuard { manager: self, seq };

        self.send_key_request(seq, track, file)
            .map_err(|error| RequestError { retryable: false, error })?;
        match tokio::time::timeout(KEY_RESPONSE_TIMEOUT, rx).await {
            Err(_) => {
                error!("Audio key response timeout");
                Err(RequestError { retryable: true, error: AudioKeyError::Timeout.into() })
            }
            Ok(Err(_)) => Err(RequestError { retryable: true, error: AudioKeyError::Channel.into() }),
            Ok(Ok(Ok(key))) => Ok(key),
            Ok(Ok(Err(error))) => {
                let retryable = matches!(
                    error.error.downcast_ref::<AudioKeyError>(),
                    Some(AudioKeyError::AesKey(AUDIO_KEY_ERROR_TRANSIENT))
                );
                Err(RequestError { retryable, error })
            }
        }
    }

    fn send_key_request(&self, seq: u32, track: SpotifyId, file: FileId) -> Result<(), Error> {
        let mut data: Vec<u8> = Vec::new();
        data.write_all(&file.0)?;
        data.write_all(&track.to_raw())?;
        data.write_u32::<BigEndian>(seq)?;
        data.write_u16::<BigEndian>(0x0000)?;

        self.session().send_packet(PacketType::RequestKey, data)
    }
}

struct RequestError {
    retryable: bool,
    error: Error,
}
