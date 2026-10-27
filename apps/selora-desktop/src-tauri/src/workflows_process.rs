//! Windows Job Object containment for one `world.run` process tree.
//!
//! `workflows_host::WorkflowHost::world_run_inner` starts a PowerShell child
//! from an environment allowlist and captures its streams.
//! `std::process::Child::kill` stops only that one process: a grandchild
//! started with `Start-Process`/`Start-Job` keeps running, and a grandchild that
//! inherited the capture pipes also keeps the run's reader joins from
//! finishing. This module owns the containment half of that gate.
//!
//! [`ProcessJob::assign`] creates an unnamed Job Object, switches on
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and adds the already-spawned PowerShell
//! child to it. Windows puts every later descendant into the same job, and the
//! job does not grant `JOB_OBJECT_LIMIT_BREAKAWAY_OK`, so the tree cannot leave
//! it. The caller keeps the returned [`ProcessJob`] alive for the whole
//! wait/capture path and calls [`ProcessJob::terminate`] before the direct-child
//! kill and before the capture joins.
//!
//! Handle lifetime is RAII: `Drop` calls `TerminateJobObject` and then
//! `CloseHandle`, so an early return cannot leak a job handle or leave a
//! descendant behind. The explicit `TerminateJobObject` is the documented tree
//! kill; the close is the `KILL_ON_JOB_CLOSE` backstop if termination itself
//! failed or was never reached.
//!
//! # Failure policy
//!
//! A job that cannot be created, configured, or assigned is a typed
//! [`HostError`] and `world_run` fails: running the command without containment
//! would silently leave grandchildren behind, which this gate does not allow.
//! `AssignProcessToJobObject` can fail with `ERROR_ACCESS_DENIED` when the
//! Selora process already sits in a restrictive Job Object that rejects nested
//! jobs; that case gets its own message instead of a generic one.
//!
//! # Residual race
//!
//! The child is created by `std::process::Command` and assigned a moment later,
//! so a descendant spawned between `CreateProcessW` returning inside the child
//! and `AssignProcessToJobObject` completing is outside the job. `std::process`
//! exposes no Windows spawn hook (no `CREATE_SUSPENDED` handshake as on Unix's
//! `pre_exec`), so that gap cannot be closed from here. In practice PowerShell
//! has not finished loading in that window. Every process created after the
//! assignment is contained.

use selora_workflow::HostError;

/// A Job Object that contains one `world.run` process tree.
///
/// The handle stays open while this value is alive. `world_run_inner` drops it
/// only after the direct child was reaped and both capture readers joined.
pub struct ProcessJob {
    #[cfg(windows)]
    handle: windows::JobHandle,
}

impl ProcessJob {
    /// Create a kill-on-close job and add the spawned PowerShell `child` to it.
    ///
    /// `child` must still be running (the caller has not seen it exit), so the
    /// process handle is valid. On failure the caller still owns a live child
    /// and must kill it; nothing was assigned to a job that outlives this call.
    pub fn assign(child: &std::process::Child) -> Result<Self, HostError> {
        #[cfg(windows)]
        {
            let handle = windows::JobHandle::create()?;
            handle.assign(child)?;
            Ok(Self { handle })
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Err(HostError::new(
                "`world.run` process containment needs a Windows Job Object; this host cannot \
                 guarantee that descendant processes are killed, so the command was stopped",
            ))
        }
    }

    /// Terminate every process still in the job.
    ///
    /// Call this on every exit path — normal exit, timeout, cancellation and a
    /// failed wait — before the direct-child `kill` and before the capture
    /// joins. Calling it on an empty job succeeds.
    pub fn terminate(&self) -> Result<(), HostError> {
        #[cfg(windows)]
        {
            self.handle.terminate()
        }
        #[cfg(not(windows))]
        {
            Ok(())
        }
    }
}

#[cfg(windows)]
mod windows {
    //! The raw Win32 calls. Every `unsafe` block here is a single documented
    //! FFI call whose handle and pointer preconditions are established by the
    //! owning struct.

    use super::HostError;
    use std::{io, os::windows::io::AsRawHandle, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
    };

    /// `ERROR_ACCESS_DENIED`: the child is already in a job that rejects nesting.
    const ERROR_ACCESS_DENIED: i32 = 5;

    /// Exit code the OS reports for processes this job terminates. They are
    /// discarded: the caller either already read the direct child's own status
    /// or returns a timeout/cancel error.
    const JOB_EXIT_CODE: u32 = 1;

    /// Sole owner of one unnamed Job Object handle. Closing it is a tree kill
    /// because the job carries `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
    pub(super) struct JobHandle(HANDLE);

    impl JobHandle {
        pub(super) fn create() -> Result<Self, HostError> {
            // SAFETY: a null security-attribute pointer and a null name ask the
            // OS for a fresh, unnamed job with default security; the returned
            // handle is the only reference and this struct takes ownership.
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if handle.is_null() {
                return Err(job_error(
                    "cannot create the `world.run` Job Object",
                    io::Error::last_os_error(),
                ));
            }
            let job = Self(handle);
            job.enable_kill_on_close()?;
            Ok(job)
        }

        fn enable_kill_on_close(&self) -> Result<(), HostError> {
            // SAFETY: `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` is a plain C struct
            // of integers and nested plain C structs; all-zero bits is its
            // documented "no limits" value, so `zeroed` is a valid value.
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `self.0` is a live job handle owned by this struct,
            // `limits` outlives the call, the information class matches the
            // struct type, and the byte length is exactly that struct's size.
            let ok = unsafe {
                SetInformationJobObject(
                    self.0,
                    JobObjectExtendedLimitInformation,
                    ptr::from_ref(&limits).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err(job_error(
                    "cannot set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE on the `world.run` Job Object",
                    io::Error::last_os_error(),
                ));
            }
            Ok(())
        }

        pub(super) fn assign(&self, child: &std::process::Child) -> Result<(), HostError> {
            // SAFETY: both handles are live and borrowed for the call — the job
            // handle is owned by `self`, and `child` is a running process the
            // caller keeps alive. Neither is closed here.
            let ok = unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
            if ok == 0 {
                let error = io::Error::last_os_error();
                return Err(if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
                    HostError::new(format!(
                        "`world.run` cannot contain its PowerShell child: Windows refused the \
                         Job Object assignment ({error}). Selora is itself running inside a \
                         restrictive Job Object that does not allow nested jobs, so descendant \
                         processes would survive the run. The command was stopped instead of \
                         run without containment"
                    ))
                } else {
                    HostError::new(format!(
                        "`world.run` cannot add its PowerShell child to the Job Object \
                         ({error}); descendant processes could not be contained, so the command \
                         was stopped"
                    ))
                });
            }
            Ok(())
        }

        pub(super) fn terminate(&self) -> Result<(), HostError> {
            // SAFETY: `self.0` is the live job handle owned by this struct and
            // the exit code is passed by value.
            if unsafe { TerminateJobObject(self.0, JOB_EXIT_CODE) } == 0 {
                return Err(job_error(
                    "cannot terminate the `world.run` Job Object; a descendant process may survive",
                    io::Error::last_os_error(),
                ));
            }
            Ok(())
        }
    }

    impl Drop for JobHandle {
        fn drop(&mut self) {
            if self.0.is_null() {
                return;
            }
            // SAFETY: `self.0` is the handle this struct owns and has not yet
            // closed: the job is terminated first (ignoring a failure — the
            // close below still kills it through KILL_ON_JOB_CLOSE), then the
            // handle is released exactly once and the field is nulled so a
            // second drop path cannot double-close it.
            unsafe {
                TerminateJobObject(self.0, JOB_EXIT_CODE);
                CloseHandle(self.0);
            }
            self.0 = ptr::null_mut();
        }
    }

    fn job_error(message: &str, error: io::Error) -> HostError {
        HostError::new(format!("{message}: {error}"))
    }
}
