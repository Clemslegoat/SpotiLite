//! Process memory introspection, shown in the interface so the RAM budget can be
//! checked at a glance.

/// Memory figures in bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct Memory {
    /// What the Windows Task Manager shows ("private working set").
    pub private_working_set: u64,
    /// Total physical memory currently mapped, shared libraries included.
    pub working_set: u64,
}

#[cfg(windows)]
pub fn memory() -> Memory {
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
        ..Default::default()
    };
    // SAFETY: the structure is correctly sized and outlives the call; the pseudo
    // handle returned by GetCurrentProcess needs no closing.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    if ok == 0 {
        return Memory::default();
    }
    Memory {
        // PrivateWorkingSetSize is only filled on Windows 10 1809 and later.
        private_working_set: if counters.PrivateWorkingSetSize > 0 {
            counters.PrivateWorkingSetSize as u64
        } else {
            counters.PrivateUsage as u64
        },
        working_set: counters.WorkingSetSize as u64,
    }
}

/// Private working set of another process (the WebView2 processes of the official
/// engine), 0 if it cannot be read.
#[cfg(windows)]
pub fn process_memory(pid: u32) -> u64 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: the handle is checked and closed; the structure is correctly sized.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return 0;
        }
        let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
            ..Default::default()
        };
        let ok = K32GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        );
        CloseHandle(process);
        if ok == 0 {
            0
        } else if counters.PrivateWorkingSetSize > 0 {
            counters.PrivateWorkingSetSize as u64
        } else {
            counters.PrivateUsage as u64
        }
    }
}

#[cfg(not(windows))]
pub fn memory() -> Memory {
    // /proc/self/statm: size resident shared text lib data dt (in pages)
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return Memory::default();
    };
    let fields: Vec<u64> = statm.split_whitespace().filter_map(|f| f.parse().ok()).collect();
    let page = 4096;
    let resident = fields.get(1).copied().unwrap_or(0) * page;
    let shared = fields.get(2).copied().unwrap_or(0) * page;
    Memory { private_working_set: resident.saturating_sub(shared), working_set: resident }
}

/// Hands pages that are not in active use back to the system (Windows only). They
/// are transparently paged back in if needed; used when the window is minimized.
#[cfg(windows)]
pub fn trim_working_set() {
    use windows_sys::Win32::System::ProcessStatus::K32EmptyWorkingSet;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: plain Win32 call on the current process pseudo handle.
    unsafe {
        K32EmptyWorkingSet(GetCurrentProcess());
    }
}

#[cfg(not(windows))]
pub fn trim_working_set() {}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_some_memory() {
        let m = super::memory();
        assert!(m.working_set > 0);
        assert!(m.private_working_set <= m.working_set || cfg!(windows));
    }
}
