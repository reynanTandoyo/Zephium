//! The app and everything it starts enter an owned kill-on-close job before
//! the app runs its first instruction (the approach of `zephium-mcp`'s stdio
//! servers).
use std::mem::{size_of, size_of_val};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use ::windows::core::PCWSTR;
use ::windows::Win32::Foundation::{ERROR_NO_MORE_FILES, HANDLE, WAIT_TIMEOUT};
use ::windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use ::windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use ::windows::Win32::System::Threading::{
    GetProcessId, GetProcessIdOfThread, OpenThread, ResumeThread, WaitForSingleObject,
    THREAD_QUERY_LIMITED_INFORMATION, THREAD_SUSPEND_RESUME,
};
use tokio::process::Child;

pub(super) struct Job(OwnedHandle);

fn handle(owned: &OwnedHandle) -> HANDLE {
    HANDLE(owned.as_raw_handle())
}

impl Job {
    pub(super) fn new() -> Result<Self, ()> {
        // SAFETY: an unnamed job with default security; the handle is owned below.
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(|_| ())?;
        // SAFETY: `job` is a fresh handle this function exclusively owns.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(job.0) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: the pointer and size describe `limits`, which outlives the call.
        unsafe {
            SetInformationJobObject(
                handle(&job.0),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&limits) as u32,
            )
        }
        .map_err(|_| ())?;
        Ok(job)
    }

    /// Ends every process in the job.
    pub(super) fn stop(&self) {
        // SAFETY: the job handle is owned and open.
        unsafe {
            let _ = TerminateJobObject(handle(&self.0), 1);
        }
    }

    /// Puts the suspended `child` in the job, then lets it run.
    pub(super) fn assign_and_resume(&self, child: &Child) -> Result<(), ()> {
        let process = HANDLE(child.raw_handle().ok_or(())?);
        let pid = child.id().ok_or(())?;
        // SAFETY: `process` is the live child's handle, borrowed for these calls.
        if unsafe { GetProcessId(process) } != pid
            || unsafe { WaitForSingleObject(process, 0) } != WAIT_TIMEOUT
        {
            return Err(());
        }
        // SAFETY: both handles are open; the child has not run yet.
        unsafe { AssignProcessToJobObject(handle(&self.0), process) }.map_err(|_| ())?;
        let thread = suspended_thread(pid)?;
        // SAFETY: `thread` is the child's only thread, opened with resume rights.
        if unsafe { ResumeThread(handle(&thread)) } != 1 {
            self.stop();
            return Err(());
        }
        Ok(())
    }
}

/// The one thread of a process created suspended.
fn suspended_thread(pid: u32) -> Result<OwnedHandle, ()> {
    // SAFETY: a thread snapshot; the handle is owned below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }.map_err(|_| ())?;
    // SAFETY: `snapshot` is a fresh handle this function exclusively owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot.0) };
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    // SAFETY: `entry` is initialized with its size, as the API requires.
    unsafe { Thread32First(handle(&snapshot), &mut entry) }.map_err(|_| ())?;
    let mut found = None;
    let mut observed = 0;
    loop {
        observed += 1;
        if observed > 65536 || entry.dwSize < size_of::<THREADENTRY32>() as u32 {
            return Err(());
        }
        if entry.th32OwnerProcessID == pid {
            if found.is_some() {
                return Err(());
            }
            // SAFETY: opening a thread by id; the handle is owned below.
            let thread = unsafe {
                OpenThread(
                    THREAD_SUSPEND_RESUME | THREAD_QUERY_LIMITED_INFORMATION,
                    false,
                    entry.th32ThreadID,
                )
            }
            .map_err(|_| ())?;
            // SAFETY: `thread` is a fresh handle this function exclusively owns.
            let thread = unsafe { OwnedHandle::from_raw_handle(thread.0) };
            // SAFETY: `thread` is open with query rights.
            if unsafe { GetProcessIdOfThread(handle(&thread)) } != pid {
                return Err(());
            }
            found = Some(thread);
        }
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        // SAFETY: as for `Thread32First`.
        match unsafe { Thread32Next(handle(&snapshot), &mut entry) } {
            Ok(()) => {}
            Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => break,
            Err(_) => return Err(()),
        }
    }
    found.ok_or(())
}
