//! WebView2 host of the playback engine: an invisible window on its own thread,
//! with the message loop WebView2 needs (every WebView2 call and callback happens
//! on this thread).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, mpsc};

use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
    COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE, CreateCoreWebView2EnvironmentWithOptions,
    ICoreWebView2, ICoreWebView2_2, ICoreWebView2_3, ICoreWebView2_19, ICoreWebView2Controller,
    ICoreWebView2Environment, ICoreWebView2Environment8, ICoreWebView2EnvironmentOptions,
    ICoreWebView2WebResourceResponseReceivedEventArgs,
};
use webview2_com::{
    CoTaskMemPWSTR, CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, ProcessFailedEventHandler, WebMessageReceivedEventHandler,
    WebResourceResponseReceivedEventHandler,
};
use windows::Win32::Foundation::{E_POINTER, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, KillTimer,
    MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, RegisterClassW, SW_HIDE, SW_SHOW, SetTimer,
    ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_TIMER, WM_USER, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::{HSTRING, Interface, PWSTR, w};

use super::{EngineCommand, EngineEvent, HOST_NAME, PLAYER_HTML, RUNTIME_URL};

type Events = Arc<dyn Fn(EngineEvent) + Send + Sync>;

pub struct Setup {
    pub dir: PathBuf,
    pub volume: f32,
    /// Chromium switches (see `Profile`).
    pub arguments: String,
    pub commands: mpsc::Receiver<EngineCommand>,
    pub thread_id: Arc<AtomicU32>,
    pub events: Events,
}

const MEMORY_TIMER: usize = 1;

/// Wakes the message loop up so that it reads new commands.
pub fn wake(thread_id: u32) {
    if thread_id != 0 {
        // SAFETY: posting a message without pointers to a thread id; failure is harmless.
        unsafe {
            let _ = PostThreadMessageW(thread_id, WM_APP, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn run(setup: Setup) {
    // SAFETY: COM and message queue initialization for this thread only.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // Creates the thread's message queue before its id is published.
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        setup.thread_id.store(GetCurrentThreadId(), Ordering::Release);
    }
    let events = setup.events.clone();
    if let Err(message) = host(setup) {
        log::warn!("engine: {message}");
        events(EngineEvent::Failed(message));
    }
    // SAFETY: balances CoInitializeEx above.
    unsafe { CoUninitialize() };
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: standard window procedure.
    unsafe {
        if msg == WM_CLOSE {
            // Only visible in debug mode: closing it must not destroy the engine.
            let _ = ShowWindow(hwnd, SW_HIDE);
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

fn describe(error: &webview2_com::Error) -> String {
    match error {
        webview2_com::Error::WindowsError(e) if e.code().0 as u32 == 0x8007_0002 => {
            format!("Microsoft Edge WebView2 is not installed on this PC: get it from {RUNTIME_URL}")
        }
        webview2_com::Error::WindowsError(e) => format!("WebView2: {} ({:#010x})", e.message(), e.code().0),
        other => format!("WebView2: {other}"),
    }
}

fn host(setup: Setup) -> Result<(), String> {
    let Setup { dir, volume, arguments, commands, events, .. } = setup;
    let debug = std::env::var_os("SPOTILITE_WEBVIEW_DEBUG").is_some();

    // The page is served from a folder mapped to https://spotilite.example.
    let page_dir = dir.join("page");
    std::fs::create_dir_all(&page_dir).map_err(|e| format!("folder {}: {e}", page_dir.display()))?;
    crate::config::write_atomic(&page_dir.join("player.html"), PLAYER_HTML.as_bytes())
        .map_err(|e| format!("player page: {e}"))?;

    // SAFETY: plain Win32 window creation on this thread.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: w!("SpotiLiteEngine"),
            ..Default::default()
        };
        // Fails harmlessly when the class exists already (engine restarted).
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SpotiLiteEngine"),
            w!("SpotiLite · lecteur"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            720,
            480,
            None,
            None,
            Some(HINSTANCE(instance.0)),
            None,
        )
        .map_err(|e| format!("window: {e}"))?
    };

    let result = host_in_window(hwnd, &dir, &arguments, volume, &commands, &events, debug);
    // SAFETY: the window belongs to this thread.
    unsafe {
        let _ = DestroyWindow(hwnd);
    }
    result
}

fn create_environment(dir: &std::path::Path, arguments: &str) -> Result<ICoreWebView2Environment, String> {
    let options = CoreWebView2EnvironmentOptions::default();
    // SAFETY: setters of a plain Rust COM object.
    unsafe {
        options.set_additional_browser_arguments(arguments.to_string());
        options.set_enable_tracking_prevention(false);
    }
    let options: ICoreWebView2EnvironmentOptions = options.into();
    let profile = HSTRING::from(dir.join("profile").as_os_str());
    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: arguments outlive the call; the handler is a valid COM object.
            unsafe {
                CreateCoreWebView2EnvironmentWithOptions(None, &profile, &options, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }
        }),
        Box::new(move |error, environment| {
            error?;
            let _ = tx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            Ok(())
        }),
    )
    .map_err(|e| describe(&e))?;
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| describe(&webview2_com::Error::WindowsError(e)))
}

fn create_controller(
    environment: &ICoreWebView2Environment,
    hwnd: HWND,
) -> Result<ICoreWebView2Controller, String> {
    let (tx, rx) = mpsc::channel();
    let environment = environment.clone();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: the window and the handler are valid for the duration of the call.
            unsafe {
                environment
                    .CreateCoreWebView2Controller(hwnd, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }
        }),
        Box::new(move |error, controller| {
            error?;
            let _ = tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)));
            Ok(())
        }),
    )
    .map_err(|e| describe(&e))?;
    rx.recv().map_err(|e| e.to_string())?.map_err(|e| describe(&webview2_com::Error::WindowsError(e)))
}

/// `Content-Length` of a response, to count the data received.
fn content_length(args: &ICoreWebView2WebResourceResponseReceivedEventArgs) -> Option<u64> {
    // SAFETY: COM getters on a valid object; the returned string is freed by CoTaskMemPWSTR.
    unsafe {
        let headers = args.Response().ok()?.Headers().ok()?;
        let mut value = PWSTR::null();
        headers.GetHeader(w!("Content-Length"), &mut value).ok()?;
        let value = CoTaskMemPWSTR::from(value);
        value.to_string().trim().parse().ok()
    }
}

fn register_handlers(webview: &ICoreWebView2, events: &Events) -> windows::core::Result<()> {
    let mut token = 0i64;
    // SAFETY: handlers are COM objects owned by WebView2 once registered.
    unsafe {
        let send = events.clone();
        webview.add_WebMessageReceived(
            &WebMessageReceivedEventHandler::create(Box::new(move |_webview, args| {
                if let Some(args) = args {
                    let mut message = PWSTR::null();
                    if args.WebMessageAsJson(&mut message).is_ok() {
                        let message = CoTaskMemPWSTR::from(message);
                        if let Some(event) = super::parse_message(&message.to_string()) {
                            send(event);
                        }
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;

        let send = events.clone();
        webview.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_webview, args| {
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                if let Some(args) = args {
                    let _ = args.ProcessFailedKind(&mut kind);
                }
                let fatal = [
                    COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
                    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
                    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
                    COREWEBVIEW2_PROCESS_FAILED_KIND_FRAME_RENDER_PROCESS_EXITED,
                ];
                if fatal.contains(&kind) {
                    send(EngineEvent::Failed(format!("a WebView2 process stopped (kind {})", kind.0)));
                } else {
                    send(EngineEvent::Log(format!("WebView2 helper process exited (kind {})", kind.0)));
                }
                Ok(())
            })),
            &mut token,
        )?;

        if let Ok(webview2) = webview.cast::<ICoreWebView2_2>() {
            let send = events.clone();
            webview2.add_WebResourceResponseReceived(
                &WebResourceResponseReceivedEventHandler::create(Box::new(move |_webview, args| {
                    if let Some(bytes) = args.as_ref().and_then(content_length).filter(|b| *b > 0) {
                        send(EngineEvent::Bytes(bytes));
                    }
                    Ok(())
                })),
                &mut token,
            )?;
        }
    }
    Ok(())
}

/// Private memory of every WebView2 process of this engine.
fn processes_memory(environment: &ICoreWebView2Environment) -> Option<u64> {
    let environment = environment.cast::<ICoreWebView2Environment8>().ok()?;
    // SAFETY: COM getters on valid objects.
    unsafe {
        let infos = environment.GetProcessInfos().ok()?;
        let mut count = 0u32;
        infos.Count(&mut count).ok()?;
        let mut total = 0;
        for i in 0..count {
            let mut pid = 0i32;
            if let Ok(info) = infos.GetValueAtIndex(i)
                && info.ProcessId(&mut pid).is_ok()
            {
                total += crate::sys::process_memory(pid as u32);
            }
        }
        Some(total)
    }
}

fn host_in_window(
    hwnd: HWND,
    dir: &std::path::Path,
    arguments: &str,
    volume: f32,
    commands: &mpsc::Receiver<EngineCommand>,
    events: &Events,
    debug: bool,
) -> Result<(), String> {
    let environment = create_environment(dir, arguments)?;
    let controller = create_controller(&environment, hwnd)?;
    let fail = |e: windows::core::Error| describe(&webview2_com::Error::WindowsError(e));

    // SAFETY: COM calls on objects created on this thread.
    let webview = unsafe {
        controller.SetBounds(RECT { left: 0, top: 0, right: 720, bottom: 480 }).map_err(fail)?;
        controller.SetIsVisible(true).map_err(fail)?;
        let webview = controller.CoreWebView2().map_err(fail)?;
        let settings = webview.Settings().map_err(fail)?;
        let _ = settings.SetAreDevToolsEnabled(debug);
        let _ = settings.SetAreDefaultContextMenusEnabled(debug);
        let _ = settings.SetIsStatusBarEnabled(false);
        let _ = settings.SetIsZoomControlEnabled(false);
        // The page only plays audio: ask WebView2 to keep its memory low (caches
        // trimmed more eagerly; available since WebView2 1.0.2210).
        if let Ok(webview19) = webview.cast::<ICoreWebView2_19>() {
            let _ = webview19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
        }
        let mapping = webview
            .cast::<ICoreWebView2_3>()
            .map_err(|_| "WebView2 is too old: update Microsoft Edge WebView2".to_string())?;
        let folder = HSTRING::from(dir.join("page").as_os_str());
        mapping
            .SetVirtualHostNameToFolderMapping(
                &HSTRING::from(HOST_NAME),
                &folder,
                COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY_CORS,
            )
            .map_err(fail)?;
        webview
    };
    register_handlers(&webview, events).map_err(fail)?;

    let url = format!("https://{HOST_NAME}/player.html?v={:.2}", volume.clamp(0.0, 1.0));
    // SAFETY: COM calls on objects created on this thread.
    unsafe {
        webview.Navigate(&HSTRING::from(url)).map_err(fail)?;
        if debug {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = webview.OpenDevToolsWindow();
        }
        SetTimer(Some(hwnd), MEMORY_TIMER, 5000, None);
    }

    let mut last_memory = 0u64;
    let mut msg = MSG::default();
    'running: loop {
        while let Ok(command) = commands.try_recv() {
            match command.to_json() {
                // SAFETY: COM call on this thread's webview.
                Some(json) => unsafe {
                    if let Err(e) = webview.PostWebMessageAsJson(&HSTRING::from(json)) {
                        log::warn!("webview message: {e}");
                    }
                },
                None => break 'running,
            }
        }
        // SAFETY: standard message loop of this thread.
        unsafe {
            match GetMessageW(&mut msg, None, 0, 0).0 {
                -1 | 0 => break,
                _ => {}
            }
            if msg.message == WM_APP && msg.hwnd.is_invalid() {
                continue;
            }
            if msg.message == WM_TIMER && msg.hwnd == hwnd && msg.wParam.0 == MEMORY_TIMER {
                if let Some(memory) = processes_memory(&environment)
                    && memory.abs_diff(last_memory) >= 1024 * 1024
                {
                    last_memory = memory;
                    events(EngineEvent::Memory(memory));
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // SAFETY: tears down the objects created on this thread.
    unsafe {
        let _ = KillTimer(Some(hwnd), MEMORY_TIMER);
        let _ = controller.Close();
    }
    events(EngineEvent::Memory(0));
    Ok(())
}
