//! Windows containment for one `world.run` process tree.
//!
//! `workflows_host::WorkflowHost::world_run_inner` needs the whole descendant
//! tree of its PowerShell child to die with the run. `std::process::Command`
//! cannot guarantee that: it returns only after `CreateProcessW` has already
//! resumed the child, so a grandchild started in the interval before
//! `AssignProcessToJobObject` escapes the job. This module owns the spawn path
//! instead and closes that window:
//!
//! 1. create the unnamed Job Object and set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`;
//! 2. create the stdin NUL device, the stdout/stderr pipes and the
//!    handle-inheritance list;
//! 3. `CreateProcessW` with `CREATE_SUSPENDED | CREATE_NO_WINDOW`: the child
//!    exists, owns its pipes, but has not executed one instruction;
//! 4. `AssignProcessToJobObject` while the child is still suspended;
//! 5. `ResumeThread`, and only after the assignment succeeded.
//!
//! Any failure in steps 1-5 terminates the child and closes the job; a child is
//! never resumed uncontained. Windows puts every later descendant into the same
//! job, and the job does not grant `JOB_OBJECT_LIMIT_BREAKAWAY_OK`, so the tree
//! cannot leave it. The caller keeps the returned [`WorldProcess`] alive for the
//! whole wait/capture path and calls [`WorldProcess::terminate_job`] before the
//! direct-child kill and before the capture joins.
//!
//! Handle lifetime is RAII: job, process, thread, pipe and NUL handles all
//! close on every path. `JobHandle`'s `Drop` calls `TerminateJobObject` and then
//! `CloseHandle`, so an early return cannot leak a job handle or leave a
//! descendant behind.
//!
//! The environment block is built only from the caller's allowlist (never the
//! ambient environment), stdin is the NUL device, stdout and stderr are separate
//! pipes, and the command line uses the same quoting rules as
//! `std::process::Command`, so `workflow_command::invocation`'s argument
//! encoding crosses the boundary unchanged.
//!
//! # Failure policy
//!
//! A job that cannot be created, configured, assigned, or resumed is a typed
//! [`HostError`] and `world_run` fails: running the command without containment
//! would silently leave grandchildren behind, which this gate does not allow.
//! `AssignProcessToJobObject` can fail with `ERROR_ACCESS_DENIED` when the
//! Selora process already sits in a restrictive Job Object that rejects nested
//! jobs; that case gets its own message instead of a generic one.
//!
//! After the run the caller kills the job and reads its membership back
//! ([`WorldProcess::audit`]): a job that still lists a process fails the run
//! instead of reporting success with debris.
//!
//! # What the job does not cover
//!
//! A process created by an out-of-process Windows broker is not a
//! `CreateProcess` descendant of the child, is never a job member, and cannot
//! be seen by the job audit. This is reproduced by the `wmi` test below: the
//! WMI service creates the process outside the job and it survives the job
//! kill. The same shape applies to Task Scheduler and Explorer. Two limits
//! remain, stated exactly:
//!
//! * `workflow_broker::refusal` refuses the known broker launch forms in the
//!   confirmed command text before PowerShell is spawned, so those commands do
//!   not run at all. That scan is a policy, not a security boundary: it does
//!   not inspect `world.run` arguments (data), so a confirmed interpreter
//!   (`powershell`, `pwsh`, `cmd`) can carry broker code as an argument, and
//!   obfuscated PowerShell can spell a broker token differently. Such a
//!   process still escapes.
//! * `Start-Process` without `-NoNewWindow` uses `ShellExecuteEx`. For an
//!   executable that path is still a `CreateProcess` descendant and stays in
//!   the job (test-covered); a document association handled by a shell broker
//!   may not be. Use `-NoNewWindow` or a direct executable command when the
//!   distinction matters.

use selora_workflow::HostError;

#[cfg(windows)]
pub use windows::WorldProcess;

#[cfg(not(windows))]
pub use fallback::WorldProcess;

/// One read-back of the Job Object's membership after a run.
///
/// `active` is the process id list Windows still reports as assigned to the
/// job. `total_created` is the job's lifetime process count. A stopped tree
/// must leave `active` empty; [`crate::workflows_host`] fails the run when it
/// does not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobAudit {
    /// Process ids still assigned to the job.
    pub active: Vec<u32>,
    /// Every process that was ever assigned to the job.
    pub total_created: u32,
}

#[cfg(windows)]
pub(crate) use windows::ProcessTree;

#[cfg(not(windows))]
#[allow(unused_imports)] // the terminal stub needs no job object
pub(crate) use fallback::ProcessTree;

/// A non-Windows host cannot provide a Job Object, so `world.run` is refused.
#[cfg(not(windows))]
mod fallback {
    use super::{HostError, JobAudit};
    use std::{ffi::OsString, fs::File, path::Path};

    /// Placeholder that keeps `world.run` fail-closed on every other host.
    #[allow(dead_code)] // constructed only by the Windows path
    pub struct WorldProcess(());

    impl WorldProcess {
        /// Always fails: this host cannot contain a descendant process tree.
        pub fn spawn(
            _program: &str,
            _args: &[String],
            _cwd: &Path,
            _environment: &[(&str, OsString)],
        ) -> Result<Self, HostError> {
            Err(HostError::new(
                "`world.run` process containment needs a Windows Job Object; this host cannot \
                 guarantee that descendant processes are killed, so the command was stopped",
            ))
        }

        pub fn take_stdout(&mut self) -> Option<File> {
            None
        }

        pub fn take_stderr(&mut self) -> Option<File> {
            None
        }

        pub fn try_wait(&mut self) -> Result<Option<u32>, HostError> {
            Err(HostError::new(
                "`world.run` process containment needs a Windows Job Object",
            ))
        }

        pub fn terminate_job(&self) -> Result<(), HostError> {
            Ok(())
        }

        /// Always fails: this host never spawned a job.
        pub fn audit(&self) -> Result<JobAudit, HostError> {
            Err(HostError::new(
                "`world.run` process containment needs a Windows Job Object",
            ))
        }

        pub fn kill_and_reap(&mut self) {}
    }

    /// No Job Object exists on this host, so the terminal backend refuses to
    /// start a session it cannot contain.
    #[allow(dead_code)] // constructed only by the Windows path
    pub(crate) struct ProcessTree(());

    impl ProcessTree {
        pub(crate) fn create() -> Result<Self, HostError> {
            Err(HostError::new(
                "terminal process containment needs a Windows Job Object; this host cannot \
                 guarantee that descendant processes are killed, so no session was started",
            ))
        }

        pub(crate) fn assign(&self, _process: *mut std::ffi::c_void) -> Result<(), HostError> {
            Err(HostError::new(
                "terminal process containment needs a Windows Job Object",
            ))
        }

        pub(crate) fn terminate(&self) -> Result<(), HostError> {
            Ok(())
        }
    }
}

#[cfg(windows)]
mod windows {
    //! The raw Win32 calls. Every `unsafe` block here is a documented FFI call
    //! whose handle and pointer preconditions are established by the owning
    //! struct or by the enclosing spawn step.

    use super::{HostError, JobAudit};
    use std::{
        ffi::{OsStr, OsString},
        fs::File,
        io,
        os::windows::{
            ffi::OsStrExt,
            io::{FromRawHandle, RawHandle},
        },
        path::{Path, PathBuf},
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, SetHandleInformation, GENERIC_READ, HANDLE, HANDLE_FLAG_INHERIT,
            INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
        },
        Security::SECURITY_ATTRIBUTES,
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
                JobObjectBasicProcessIdList, JobObjectExtendedLimitInformation,
                QueryInformationJobObject, SetInformationJobObject, TerminateJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_BASIC_PROCESS_ID_LIST,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Pipes::CreatePipe,
            Threading::{
                CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
                InitializeProcThreadAttributeList, ResumeThread, TerminateProcess,
                UpdateProcThreadAttribute, WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED,
                CREATE_UNICODE_ENVIRONMENT, EXTENDED_STARTUPINFO_PRESENT, INFINITE,
                LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
            },
        },
    };

    // Only the containment assertions in tests query another process's job.
    #[cfg(test)]
    use windows_sys::Win32::{
        System::JobObjects::IsProcessInJob,
        System::Threading::{GetProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };

    /// `ERROR_ACCESS_DENIED`: the child is already in a job that rejects nesting.
    const ERROR_ACCESS_DENIED: i32 = 5;

    /// Exit code the OS reports for processes this module terminates. They are
    /// discarded: the caller either already read the direct child's own status
    /// or returns a timeout/cancel error.
    const JOB_EXIT_CODE: u32 = 1;

    /// The spawn-time assignment hook. Production passes
    /// [`JobHandle::assign_raw`]; tests pass a failing call to prove the child
    /// is terminated without ever being resumed.
    type AssignFn<'a> = dyn Fn(HANDLE, HANDLE) -> Result<(), HostError> + 'a;

    /// One contained `world.run` child plus the job that owns its tree.
    ///
    /// `spawn` creates the job first, starts the child suspended, assigns it to
    /// the job, and resumes it only after the assignment. The job handle stays
    /// open while this value is alive, so descendants cannot outlive it.
    pub struct WorldProcess {
        job: JobHandle,
        process: ProcessHandle,
        stdout: Option<File>,
        stderr: Option<File>,
    }

    impl WorldProcess {
        /// Spawn `program` suspended, contain it in a kill-on-close job, then
        /// resume it. `args` are passed as separate argv entries; `cwd` and
        /// `environment` replace the child's inherited values completely.
        pub fn spawn(
            program: &str,
            args: &[String],
            cwd: &Path,
            environment: &[(&str, OsString)],
        ) -> Result<Self, HostError> {
            spawn_with_assign(program, args, cwd, environment, &JobHandle::assign_raw)
        }

        /// The child's stdout read end, if the spawn created it.
        pub fn take_stdout(&mut self) -> Option<File> {
            self.stdout.take()
        }

        /// The child's stderr read end, if the spawn created it.
        pub fn take_stderr(&mut self) -> Option<File> {
            self.stderr.take()
        }

        /// Poll for exit without blocking. `Ok(Some(code))` is the child's own
        /// exit code; it is never the job's synthetic code.
        pub fn try_wait(&mut self) -> Result<Option<u32>, HostError> {
            // SAFETY: `self.process` is a live process handle owned by this
            // value; a zero timeout polls without blocking.
            match unsafe { WaitForSingleObject(self.process.raw(), 0) } {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut code = 0u32;
                    // SAFETY: the process handle is valid and `code` is a live
                    // `u32` out-parameter.
                    if unsafe { GetExitCodeProcess(self.process.raw(), &mut code) } == 0 {
                        return Err(HostError::new(format!(
                            "cannot read the `world.run` child's exit code: {}",
                            io::Error::last_os_error()
                        )));
                    }
                    Ok(Some(code))
                }
                _ => Err(HostError::new(format!(
                    "cannot wait for the `world.run` child: {}",
                    io::Error::last_os_error()
                ))),
            }
        }

        /// Terminate every process still in the job.
        ///
        /// Call this on every exit path — normal exit, timeout, cancellation and
        /// a failed wait — before [`Self::kill_and_reap`] and before the capture
        /// joins. Calling it on an empty job succeeds.
        pub fn terminate_job(&self) -> Result<(), HostError> {
            self.job.terminate()
        }

        /// Read the job's current membership back.
        ///
        /// Call after [`Self::terminate_job`]. `active` must be empty before
        /// the run can be reported as successful; a non-empty list means at
        /// least one process the job owns is still alive.
        ///
        /// This sees job members only. A process a Windows broker created
        /// outside the job is not a member and cannot appear here; that is the
        /// documented residual, and why the host also refuses known broker
        /// launch forms before spawning.
        pub fn audit(&self) -> Result<JobAudit, HostError> {
            self.job.audit()
        }

        /// Kill and reap the direct child as the belt-and-braces fallback.
        ///
        /// The job kill above is authoritative; this only guarantees the direct
        /// handle is reaped even when the job termination call failed.
        pub fn kill_and_reap(&mut self) {
            // SAFETY: both calls use the live process handle owned by this
            // value. A process that already exited makes them fail harmlessly,
            // which is why the results are ignored exactly as the previous
            // `Child::kill`/`Child::wait` fallback ignored them.
            unsafe {
                TerminateProcess(self.process.raw(), JOB_EXIT_CODE);
                WaitForSingleObject(self.process.raw(), INFINITE);
            }
        }

        /// The Windows process id of the contained child.
        #[cfg(test)]
        pub(super) fn pid(&self) -> u32 {
            self.process.id()
        }

        /// Whether `pid` is currently a member of this child's job.
        ///
        /// Test-only proof that a grandchild spawned by the child itself was
        /// contained at creation time rather than killed after the fact.
        #[cfg(test)]
        pub(super) fn job_contains(&self, pid: u32) -> bool {
            // SAFETY: `OpenProcess` takes a plain access mask and process id;
            // the returned handle is checked before use. The job handle is the
            // live handle owned by this value.
            let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
            if !valid(process) {
                return false;
            }
            let process = ProcessHandle(process);
            let mut member = 0i32;
            // SAFETY: both handles are valid for the call and `member` is a
            // live `BOOL` out-parameter.
            let ok = unsafe { IsProcessInJob(process.raw(), self.job.raw(), &mut member) };
            ok != 0 && member != 0
        }
    }

    /// One standalone kill-on-close Job Object for a child another module
    /// spawned (the ConPTY terminal). It owns only the job, not the child
    /// handle: the terminal keeps its own `portable-pty` child handle for
    /// waiting, and the job is what makes a close a *tree* kill.
    ///
    /// `assign` must run immediately after the spawn so the shell is a member
    /// before it can start a descendant. A failed assignment fails the whole
    /// create: the terminal backend kills the child instead of running it
    /// uncontained, the same fail-closed rule `WorldProcess::spawn` follows.
    pub(crate) struct ProcessTree {
        job: JobHandle,
    }

    impl ProcessTree {
        /// A fresh unnamed job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` set.
        pub(crate) fn create() -> Result<Self, HostError> {
            Ok(Self {
                job: JobHandle::create()?,
            })
        }

        /// Add a live child process (its process handle) to the job.
        pub(crate) fn assign(&self, process: RawHandle) -> Result<(), HostError> {
            JobHandle::assign_raw(self.job.raw(), process as HANDLE)
        }

        /// Terminate every process still in the job; an empty job succeeds.
        pub(crate) fn terminate(&self) -> Result<(), HostError> {
            self.job.terminate()
        }
    }

    // SAFETY: a Job Object handle is a kernel object handle, not thread-bound
    // state. `AssignProcessToJobObject`, `TerminateJobObject` and `CloseHandle`
    // are valid from any thread, the job is immutable after `create`, and the
    // only mutable access is the handle release in `JobHandle::drop` (which
    // runs exactly once, when the last owner is dropped). Sharing `&self`
    // across threads only ever issues those documented, serialize-in-the-OS
    // calls.
    unsafe impl Send for ProcessTree {}
    unsafe impl Sync for ProcessTree {}

    /// The real spawn path; `assign` exists so a test can force the assignment
    /// step to fail without weakening the production call.
    pub(super) fn spawn_with_assign(
        program: &str,
        args: &[String],
        cwd: &Path,
        environment: &[(&str, OsString)],
        assign: &AssignFn<'_>,
    ) -> Result<WorldProcess, HostError> {
        let application = resolve_program(program)?;
        // The job exists before the process does, so there is no instant at
        // which a live child has no job to be assigned to.
        let job = JobHandle::create()?;
        let stdio = StdioHandles::create()?;

        let mut command_line = command_line(application.as_os_str(), args);
        let environment_block = environment_block(environment);
        let current_directory = current_directory(cwd);
        let application: Vec<u16> = application
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // The handle values must outlive both `UpdateProcThreadAttribute` and
        // `CreateProcessW`, so keep the array in this frame.
        let child_handles = stdio.child_handles();
        let attributes = AttributeList::new(&child_handles)?;

        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = stdio.nul.raw();
        startup.StartupInfo.hStdOutput = stdio.out_write.raw();
        startup.StartupInfo.hStdError = stdio.err_write.raw();
        startup.lpAttributeList = attributes.raw();

        let mut process_info = PROCESS_INFORMATION::default();
        // SAFETY: every pointer is valid for the duration of the call and lives
        // in this frame: the mutable command-line, NUL-terminated application,
        // environment and current-directory buffers, the extended startup info
        // with its initialized attribute list, and the writable process-info
        // out-parameter. The attribute list restricts inheritance to the three
        // standard handles above, so no other inheritable handle leaks in.
        let created = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                1,
                CREATE_SUSPENDED
                    | CREATE_NO_WINDOW
                    | EXTENDED_STARTUPINFO_PRESENT
                    | CREATE_UNICODE_ENVIRONMENT,
                environment_block.as_ptr().cast(),
                current_directory.as_ptr(),
                &startup.StartupInfo,
                &mut process_info,
            )
        };
        if created == 0 {
            return Err(HostError::new(format!(
                "cannot create the suspended `world.run` PowerShell child (`{program}`): {}",
                io::Error::last_os_error()
            )));
        }
        // SAFETY: on success `CreateProcessW` returned two distinct owned
        // handles. Each wrapper below takes one and closes it exactly once.
        let process = ProcessHandle(process_info.hProcess);
        let thread = OwnedHandle(process_info.hThread);

        // The child is suspended, so no descendant can exist yet. Assignment is
        // therefore atomic with respect to "before the child executes".
        if let Err(error) = assign(job.raw(), process.raw()) {
            // Never resume an uncontained child: terminate and reap it, then
            // let the job and pipe handles close on the way out.
            terminate_and_wait(process.raw());
            return Err(error);
        }

        // SAFETY: `thread` is the live primary thread handle returned by
        // CreateProcessW for the suspended child.
        if unsafe { ResumeThread(thread.raw()) } == u32::MAX {
            let error = io::Error::last_os_error();
            terminate_and_wait(process.raw());
            return Err(HostError::new(format!(
                "cannot resume the contained `world.run` child: {error}; the child was \
                 terminated without running"
            )));
        }
        drop(thread);

        let stdout = stdio.out_read.into_file();
        let stderr = stdio.err_read.into_file();
        // `stdio` now drops its write and NUL ends, so the capture readers see
        // EOF when the last descendant holding an inherited pipe exits.
        Ok(WorldProcess {
            job,
            process,
            stdout: Some(stdout),
            stderr: Some(stderr),
        })
    }

    /// Resolve a bare program name against `PATH` before `CreateProcessW`.
    ///
    /// Passing the resolved path as `lpApplicationName` keeps the search order
    /// identical to `std::process::Command` (which does not search the current
    /// directory); leaving it null would add the parent's working directory to
    /// the search and let a planted `pwsh.exe` win.
    fn resolve_program(program: &str) -> Result<PathBuf, HostError> {
        let path = Path::new(program);
        if path.is_absolute() || program.contains(['\\', '/']) {
            return Ok(path.to_path_buf());
        }
        let mut searched = false;
        if let Some(directories) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&directories) {
                let candidate = directory.join(program);
                if candidate.is_file() {
                    return Ok(candidate);
                }
                let candidate = directory.join(format!("{program}.exe"));
                if candidate.is_file() {
                    return Ok(candidate);
                }
                searched = true;
            }
        }
        Err(HostError::new(if searched {
            format!("cannot find `{program}` on PATH")
        } else {
            format!("cannot find `{program}` because PATH is not set")
        }))
    }

    /// The child's NUL-terminated working directory.
    ///
    /// `Path::canonicalize` (and therefore the host's workspace root) returns
    /// the `\\?\` verbatim form on Windows. Passing that form to
    /// `CreateProcessW` makes PowerShell report `Microsoft.PowerShell.Core\
    /// FileSystem::\\?\…` from `(Get-Location).Path`, whereas
    /// `std::process::Command` passes the plain form. Strip the prefix (and map
    /// `\\?\UNC\` back to `\\`) so `world.run` keeps the previous behaviour.
    pub(super) fn current_directory(cwd: &Path) -> Vec<u16> {
        const BACKSLASH: u16 = b'\\' as u16;
        let mut units: Vec<u16> = cwd.as_os_str().encode_wide().collect();
        let verbatim = [BACKSLASH, BACKSLASH, u16::from(b'?'), BACKSLASH];
        if units.starts_with(&verbatim) {
            let unc = [u16::from(b'U'), u16::from(b'N'), u16::from(b'C'), BACKSLASH];
            if units[verbatim.len()..].starts_with(&unc) {
                let mut normalized = vec![BACKSLASH; 2];
                normalized.extend_from_slice(&units[verbatim.len() + unc.len()..]);
                units = normalized;
            } else {
                units.drain(..verbatim.len());
            }
        }
        units.push(0);
        units
    }

    /// Terminate a suspended (or just-failed) child and wait for it to be gone.
    fn terminate_and_wait(process: HANDLE) {
        // SAFETY: `process` is a live process handle owned by the caller for
        // this call. Failures are possible when the process already exited and
        // are intentionally ignored; the caller reports its own error.
        unsafe {
            TerminateProcess(process, JOB_EXIT_CODE);
            WaitForSingleObject(process, INFINITE);
        }
    }

    /// Sole owner of one unnamed Job Object handle. Closing it is a tree kill
    /// because the job carries `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
    struct JobHandle(HANDLE);

    impl JobHandle {
        fn create() -> Result<Self, HostError> {
            // SAFETY: a null security-attribute pointer and a null name ask the
            // OS for a fresh, unnamed job with default security; the returned
            // handle is the only reference and this struct takes ownership.
            let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
            if !valid(handle) {
                return Err(job_error(
                    "cannot create the process Job Object",
                    io::Error::last_os_error(),
                ));
            }
            let job = Self(handle);
            job.enable_kill_on_close()?;
            Ok(job)
        }

        fn raw(&self) -> HANDLE {
            self.0
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
                    "cannot set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE on the process Job Object",
                    io::Error::last_os_error(),
                ));
            }
            Ok(())
        }

        /// Add one process to `job`. The process may be suspended: the kernel
        /// job link is what `ResumeThread` is gated on.
        fn assign_raw(job: HANDLE, process: HANDLE) -> Result<(), HostError> {
            // SAFETY: both handles are live and borrowed for the call — the job
            // handle is owned by the caller, and the process handle belongs to
            // the just-created suspended child. Neither is closed here.
            let ok = unsafe { AssignProcessToJobObject(job, process) };
            if ok == 0 {
                let error = io::Error::last_os_error();
                return Err(if error.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
                    HostError::new(format!(
                        "cannot contain the child process: Windows refused the Job Object \
                         assignment ({error}). Selora is itself running inside a restrictive \
                         Job Object that does not allow nested jobs, so descendant processes \
                         would survive"
                    ))
                } else {
                    HostError::new(format!(
                        "cannot add the child process to the Job Object ({error}); descendant \
                         processes could not be contained"
                    ))
                });
            }
            Ok(())
        }

        fn terminate(&self) -> Result<(), HostError> {
            // SAFETY: `self.0` is the live job handle owned by this struct and
            // the exit code is passed by value.
            if unsafe { TerminateJobObject(self.0, JOB_EXIT_CODE) } == 0 {
                return Err(job_error(
                    "cannot terminate the process Job Object; a descendant process may survive",
                    io::Error::last_os_error(),
                ));
            }
            Ok(())
        }

        /// Query the job's lifetime process count and its current pid list.
        fn audit(&self) -> Result<JobAudit, HostError> {
            let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION =
                // SAFETY: the struct is a plain C struct of integers; all-zero
                // bits is a valid value that the call overwrites.
                unsafe { std::mem::zeroed() };
            let mut returned = 0u32;
            // SAFETY: `self.0` is a live job handle; `accounting` is a live,
            // correctly sized out-parameter for the basic accounting class.
            if unsafe {
                QueryInformationJobObject(
                    self.0,
                    JobObjectBasicAccountingInformation,
                    ptr::from_mut(&mut accounting).cast(),
                    std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    &mut returned,
                )
            } == 0
            {
                return Err(job_error(
                    "cannot read the `world.run` Job Object accounting",
                    io::Error::last_os_error(),
                ));
            }

            // `NumberOfAssignedProcesses` can exceed the buffer, in which case
            // the call succeeds with a truncated list. Grow to the reported
            // size until the list is complete.
            let mut capacity = 64usize;
            loop {
                let header = std::mem::size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
                let extra = capacity.saturating_sub(1) * std::mem::size_of::<usize>();
                let bytes = header + extra;
                // `ProcessIdList` is a one-element array; a `usize` buffer keeps
                // the allocation pointer-aligned for the pid array.
                let mut storage = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
                let list = storage
                    .as_mut_ptr()
                    .cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>();
                let mut returned = 0u32;
                // SAFETY: `self.0` is a live job handle; `list` points at
                // `bytes` writable bytes of aligned storage; the class matches
                // the struct and `returned` is a live out-parameter.
                if unsafe {
                    QueryInformationJobObject(
                        self.0,
                        JobObjectBasicProcessIdList,
                        list.cast(),
                        bytes as u32,
                        &mut returned,
                    )
                } == 0
                {
                    return Err(job_error(
                        "cannot read the `world.run` Job Object process list",
                        io::Error::last_os_error(),
                    ));
                }
                // SAFETY: the successful call initialized the header and at
                // most `capacity` pid entries inside `storage`.
                let (assigned, listed) = unsafe {
                    (
                        (*list).NumberOfAssignedProcesses as usize,
                        (*list).NumberOfProcessIdsInList as usize,
                    )
                };
                if assigned > capacity {
                    capacity = assigned;
                    continue;
                }
                // SAFETY: `listed` is at most the requested `capacity`, and the
                // entries follow the struct header inside `storage`.
                let active = unsafe {
                    std::slice::from_raw_parts((*list).ProcessIdList.as_ptr(), listed)
                        .iter()
                        .map(|pid| *pid as u32)
                        .collect()
                };
                return Ok(JobAudit {
                    active,
                    total_created: accounting.TotalProcesses,
                });
            }
        }
    }

    impl Drop for JobHandle {
        fn drop(&mut self) {
            if !valid(self.0) {
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

    /// Owned process handle; `Drop` closes it once.
    struct ProcessHandle(HANDLE);

    impl ProcessHandle {
        fn raw(&self) -> HANDLE {
            self.0
        }

        #[cfg(test)]
        fn id(&self) -> u32 {
            // SAFETY: the handle is a live process handle owned by this struct.
            unsafe { GetProcessId(self.0) }
        }
    }

    impl Drop for ProcessHandle {
        fn drop(&mut self) {
            if valid(self.0) {
                // SAFETY: this struct owns the handle and closes it once.
                unsafe { CloseHandle(self.0) };
                self.0 = ptr::null_mut();
            }
        }
    }

    /// Owned non-process kernel handle (thread, pipe end, NUL device).
    struct OwnedHandle(HANDLE);

    impl OwnedHandle {
        fn new(handle: HANDLE) -> Option<Self> {
            valid(handle).then_some(Self(handle))
        }

        fn raw(&self) -> HANDLE {
            self.0
        }

        /// Hand the handle to [`File`] so the capture readers can use it.
        fn into_file(mut self) -> File {
            let handle = self.0;
            self.0 = ptr::null_mut();
            // SAFETY: `handle` was owned by this value and is forgotten here, so
            // the returned `File` is its only owner. It is a pipe read end,
            // which `File::read` accepts.
            unsafe { File::from_raw_handle(handle as RawHandle) }
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if valid(self.0) {
                // SAFETY: this struct owns the handle and closes it once.
                unsafe { CloseHandle(self.0) };
                self.0 = ptr::null_mut();
            }
        }
    }

    /// The three inheritable standard handles and the two parent read ends.
    struct StdioHandles {
        nul: OwnedHandle,
        out_write: OwnedHandle,
        err_write: OwnedHandle,
        out_read: OwnedHandle,
        err_read: OwnedHandle,
    }

    impl StdioHandles {
        fn create() -> Result<Self, HostError> {
            let attributes = inheritable_security_attributes();
            let nul = create_nul(&attributes)?;
            let (out_read, out_write) = create_pipe(&attributes)?;
            let (err_read, err_write) = create_pipe(&attributes)?;
            // The parent keeps the read ends across later spawns; they must not
            // become inherited by an unrelated child.
            for handle in [out_read.raw(), err_read.raw()] {
                // SAFETY: both handles are live and owned by the locals above.
                if unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) } == 0 {
                    return Err(HostError::new(format!(
                        "cannot make the `world.run` capture pipe private: {}",
                        io::Error::last_os_error()
                    )));
                }
            }
            Ok(Self {
                nul,
                out_write,
                err_write,
                out_read,
                err_read,
            })
        }

        /// The handles the child inherits, in stdin/stdout/stderr order.
        fn child_handles(&self) -> [HANDLE; 3] {
            [self.nul.raw(), self.out_write.raw(), self.err_write.raw()]
        }
    }

    /// A `SECURITY_ATTRIBUTES` that makes a created handle inheritable.
    fn inheritable_security_attributes() -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: 1,
        }
    }

    fn create_nul(attributes: &SECURITY_ATTRIBUTES) -> Result<OwnedHandle, HostError> {
        let name: Vec<u16> = "NUL\0".encode_utf16().collect();
        // SAFETY: `name` is a NUL-terminated UTF-16 path that outlives the
        // call, and `attributes` is a valid struct for the call. `NUL` opens
        // the null device, so the child's stdin is always at EOF.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                attributes,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        OwnedHandle::new(handle).ok_or_else(|| {
            HostError::new(format!(
                "cannot open the `world.run` child's stdin NUL device: {}",
                io::Error::last_os_error()
            ))
        })
    }

    fn create_pipe(
        attributes: &SECURITY_ATTRIBUTES,
    ) -> Result<(OwnedHandle, OwnedHandle), HostError> {
        let mut read = ptr::null_mut();
        let mut write = ptr::null_mut();
        // SAFETY: both out-parameters are live `HANDLE` slots, and `attributes`
        // marks the new handles inheritable. A zero size asks for the default
        // buffer size.
        if unsafe { CreatePipe(&mut read, &mut write, attributes, 0) } == 0 {
            return Err(HostError::new(format!(
                "cannot create a `world.run` capture pipe: {}",
                io::Error::last_os_error()
            )));
        }
        let read = OwnedHandle::new(read);
        let write = OwnedHandle::new(write);
        match (read, write) {
            (Some(read), Some(write)) => Ok((read, write)),
            _ => Err(HostError::new(
                "Windows returned an invalid `world.run` capture pipe handle",
            )),
        }
    }

    /// `InitializeProcThreadAttributeList` output kept alive for the spawn.
    ///
    /// The list records a pointer to the caller's handle array, so the caller
    /// keeps that array (`child_handles`) alive until `CreateProcessW` returns.
    struct AttributeList {
        // Backing storage for the opaque list; it must stay pointer-aligned and
        // must outlive `ptr`.
        _storage: Vec<usize>,
        ptr: LPPROC_THREAD_ATTRIBUTE_LIST,
    }

    impl AttributeList {
        fn new(handles: &[HANDLE]) -> Result<Self, HostError> {
            let mut size = 0usize;
            // SAFETY: a null list with a valid size out-parameter is the
            // documented sizing probe; it fails with ERROR_INSUFFICIENT_BUFFER.
            unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut size) };
            if size == 0 {
                return Err(attribute_error(io::Error::last_os_error()));
            }
            let words = size.div_ceil(std::mem::size_of::<usize>());
            let mut storage = vec![0usize; words];
            let ptr: LPPROC_THREAD_ATTRIBUTE_LIST = storage.as_mut_ptr().cast();
            // SAFETY: `ptr` points at `storage`, which is at least `size` bytes
            // and pointer-aligned (Vec<usize>), and one attribute slot was
            // requested in the probe above. A failure here leaves the list
            // uninitialized, so it is not wrapped in `AttributeList` and no
            // delete runs.
            if unsafe { InitializeProcThreadAttributeList(ptr, 1, 0, &mut size) } == 0 {
                return Err(attribute_error(io::Error::last_os_error()));
            }
            let list = Self {
                _storage: storage,
                ptr,
            };
            // SAFETY: the list is initialized with one slot reserved. The value
            // points at `handles`, which the caller keeps alive through
            // `CreateProcessW`, and its byte length is exact. No previous value
            // or return size is needed.
            let ok = unsafe {
                UpdateProcThreadAttribute(
                    list.ptr,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    handles.as_ptr().cast(),
                    std::mem::size_of_val(handles),
                    ptr::null_mut(),
                    ptr::null(),
                )
            };
            if ok == 0 {
                return Err(attribute_error(io::Error::last_os_error()));
            }
            Ok(list)
        }

        fn raw(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
            self.ptr
        }
    }

    impl Drop for AttributeList {
        fn drop(&mut self) {
            if !self.ptr.is_null() {
                // SAFETY: `ptr` was initialized by `Self::new` and has not been
                // deleted yet; `_storage` still backs it and is dropped after
                // this call.
                unsafe { DeleteProcThreadAttributeList(self.ptr) };
                self.ptr = ptr::null_mut();
            }
        }
    }

    fn attribute_error(error: io::Error) -> HostError {
        HostError::new(format!(
            "cannot restrict `world.run` handle inheritance: {error}"
        ))
    }

    /// A Windows command line that round-trips through `CommandLineToArgvW`,
    /// using the same quoting rules as `std::process::Command`.
    fn command_line(program: &OsStr, args: &[String]) -> Vec<u16> {
        let mut line = Vec::new();
        push_quoted(&mut line, program);
        for argument in args {
            line.push(u16::from(b' '));
            push_quoted(&mut line, OsStr::new(argument));
        }
        line.push(0);
        line
    }

    fn push_quoted(line: &mut Vec<u16>, argument: &OsStr) {
        const SPACE: u16 = b' ' as u16;
        const TAB: u16 = b'\t' as u16;
        const QUOTE: u16 = b'"' as u16;
        const BACKSLASH: u16 = b'\\' as u16;

        let units: Vec<u16> = argument.encode_wide().collect();
        let needs_quotes = units.is_empty()
            || units
                .iter()
                .any(|unit| matches!(*unit, SPACE | TAB | QUOTE | 10 | 11 | 13));
        if !needs_quotes {
            line.extend_from_slice(&units);
            return;
        }
        line.push(QUOTE);
        let mut backslashes = 0usize;
        for unit in units {
            if unit == BACKSLASH {
                backslashes += 1;
                continue;
            }
            if unit == QUOTE {
                // 2n+1 backslashes: n literal ones plus one escaping the quote.
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2 + 1));
            } else {
                line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
            }
            line.push(unit);
            backslashes = 0;
        }
        // Trailing backslashes precede the closing quote and must be doubled.
        line.extend(std::iter::repeat_n(BACKSLASH, backslashes * 2));
        line.push(QUOTE);
    }

    /// A UTF-16, NUL-terminated, case-insensitively sorted environment block.
    ///
    /// Only `environment` entries are present: the child never inherits the
    /// parent's ambient environment (and therefore never the gateway key).
    fn environment_block(environment: &[(&str, OsString)]) -> Vec<u16> {
        let mut entries: Vec<(&str, &OsString)> =
            environment.iter().map(|(n, v)| (*n, v)).collect();
        entries.sort_by_key(|(name, _value)| name.to_ascii_uppercase());
        let mut block = Vec::new();
        for (name, value) in entries {
            block.extend(name.encode_utf16());
            block.push(u16::from(b'='));
            block.extend(value.encode_wide());
            block.push(0);
        }
        block.push(0);
        block
    }

    fn valid(handle: HANDLE) -> bool {
        !handle.is_null() && handle != INVALID_HANDLE_VALUE
    }

    fn job_error(message: &str, error: io::Error) -> HostError {
        HostError::new(format!("{message}: {error}"))
    }

    /// Test-only: `AssignProcessToJobObject` against a deliberately invalid job
    /// handle, so the failure test drives the real Win32 error path.
    #[cfg(test)]
    pub(super) fn assign_against_invalid_job(process: HANDLE) -> Result<(), HostError> {
        JobHandle::assign_raw(ptr::null_mut(), process)
    }

    /// Test-only: the pid behind a raw process handle.
    #[cfg(test)]
    pub(super) fn raw_process_id(process: HANDLE) -> u32 {
        // SAFETY: the caller passes the live handle of a process it just
        // spawned; `GetProcessId` does not take ownership.
        unsafe { GetProcessId(process) }
    }
}

#[cfg(all(test, windows))]
mod tests {
    //! These tests make the child spawn its grandchild as its very first
    //! statement, so they fail if assignment ever happens after the child can
    //! execute.

    use super::{
        windows::{assign_against_invalid_job, raw_process_id, spawn_with_assign},
        WorldProcess,
    };
    use std::{
        ffi::OsString,
        fs,
        path::{Path, PathBuf},
        process::{Command, Stdio},
        sync::Mutex,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    fn powershell() -> &'static str {
        if Command::new("pwsh")
            .args(["-NoProfile", "-Command", "exit 0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            "pwsh"
        } else {
            "powershell"
        }
    }

    fn environment() -> Vec<(&'static str, OsString)> {
        [
            "PATH",
            "SystemRoot",
            "TEMP",
            "TMP",
            "APPDATA",
            "LOCALAPPDATA",
            "USERPROFILE",
        ]
        .into_iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (name, value)))
        .collect()
    }

    fn args(script: &str) -> Vec<String> {
        vec![
            "-NoProfile".to_owned(),
            "-NonInteractive".to_owned(),
            "-Command".to_owned(),
            script.to_owned(),
        ]
    }

    /// A PowerShell single-quoted literal for `value` (`'` is doubled).
    fn ps_literal(value: &Path) -> String {
        format!("'{}'", value.display().to_string().replace('\'', "''"))
    }

    fn wait_for_pid_file(path: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(text) = fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse() {
                    return pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "the grandchild never wrote its PID to {path:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Non-panicking variant for the broker probe: WMI may be disabled on a
    /// host, in which case there is no broker attempt to reproduce.
    fn wait_for_pid_file_opt(path: &Path) -> Option<u32> {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(text) = fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse() {
                    return Some(pid);
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Terminate a process the job does not own, for broker-test cleanup.
    fn terminate_outside_process(pid: u32) {
        // SAFETY: `OpenProcess` takes a plain access mask and process id; the
        // returned handle is checked and closed. `TerminateProcess` takes the
        // live handle and a plain exit code.
        unsafe {
            let handle = OpenProcess(
                PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            );
            if !handle.is_null() {
                TerminateProcess(handle, 1);
                windows_sys::Win32::Foundation::CloseHandle(handle);
            }
        }
    }

    fn process_alive(pid: u32) -> bool {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .stdin(Stdio::null())
            .output()
            .expect("tasklist runs");
        String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
    }

    fn wait_until_gone(pid: u32) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if !process_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        !process_alive(pid)
    }

    /// The script's first statement starts a second PowerShell (a grandchild
    /// the direct-child kill cannot reach) and records its PID. Only then does
    /// it do anything else.
    fn grandchild_first_script(pid_file: &Path, tail: &str) -> String {
        format!(
            "$grand = Start-Process -FilePath {} -ArgumentList '-NoProfile',\
             '-NonInteractive','-Command','Start-Sleep -Seconds 120' -NoNewWindow -PassThru; \
             Set-Content -LiteralPath {} -Value $grand.Id; {tail}",
            ps_literal(Path::new(powershell())),
            ps_literal(pid_file)
        )
    }

    /// The same grandchild through `Start-Process`'s default `ShellExecuteEx`
    /// path (no `-NoNewWindow`). For an executable, Windows still creates it
    /// as a `CreateProcess` descendant, so it must stay in the job.
    fn shell_execute_grandchild_script(pid_file: &Path) -> String {
        format!(
            "$grand = Start-Process -FilePath {} -ArgumentList '-NoProfile',\
             '-NonInteractive','-Command','Start-Sleep -Seconds 120' -PassThru; \
             Set-Content -LiteralPath {} -Value $grand.Id; Start-Sleep -Seconds 120",
            ps_literal(Path::new(powershell())),
            ps_literal(pid_file)
        )
    }

    /// A grandchild created by the WMI service (`WmiPrvSE.exe`), not by the
    /// child. This is the broker escape: the process is outside the job, so
    /// neither the job kill nor the job audit can reach it.
    fn wmi_broker_script(pid_file: &Path) -> String {
        format!(
            "$cmd = '{} -NoProfile -Command \"Start-Sleep -Seconds 120\"'; \
             $r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create \
             -Arguments @{{CommandLine=$cmd}}; \
             Set-Content -LiteralPath {} -Value $r.ProcessId; Start-Sleep -Seconds 120",
            powershell(),
            ps_literal(pid_file)
        )
    }

    #[test]
    fn current_directory_drops_the_verbatim_prefix() {
        let units = super::windows::current_directory(Path::new(r"\\?\C:\work\repo"));
        let text = String::from_utf16(&units[..units.len() - 1]).expect("utf-16");
        assert_eq!(text, r"C:\work\repo");

        let units = super::windows::current_directory(Path::new(r"\\?\UNC\server\share\repo"));
        let text = String::from_utf16(&units[..units.len() - 1]).expect("utf-16");
        assert_eq!(text, r"\\server\share\repo");

        let units = super::windows::current_directory(Path::new(r"C:\plain\repo"));
        let text = String::from_utf16(&units[..units.len() - 1]).expect("utf-16");
        assert_eq!(text, r"C:\plain\repo");
    }

    #[test]
    fn child_spawned_grandchild_is_in_the_job_and_dies_with_it() {
        for attempt in 0..3 {
            let workspace = tempfile::tempdir().expect("workspace");
            let pid_file: PathBuf = workspace.path().join(format!("grandchild-{attempt}.pid"));
            let script = grandchild_first_script(&pid_file, "Start-Sleep -Seconds 120");
            let mut child = WorldProcess::spawn(
                powershell(),
                &args(&script),
                workspace.path(),
                &environment(),
            )
            .expect("suspended spawn and assignment succeed");
            let grandchild = wait_for_pid_file(&pid_file);
            assert!(
                child.job_contains(child.pid()),
                "attempt {attempt}: the child itself is not in its job"
            );
            assert!(
                child.job_contains(grandchild),
                "attempt {attempt}: grandchild {grandchild} spawned before assignment was in the \
                 job; the suspend window is open"
            );
            child.terminate_job().expect("job terminates");
            child.kill_and_reap();
            assert!(
                wait_until_gone(grandchild),
                "attempt {attempt}: grandchild {grandchild} survived the job termination"
            );
        }
    }

    #[test]
    fn start_process_shell_execute_grandchild_is_in_the_job_and_dies_with_it() {
        let workspace = tempfile::tempdir().expect("workspace");
        let pid_file = workspace.path().join("shell-execute.pid");
        let script = shell_execute_grandchild_script(&pid_file);
        let mut child = WorldProcess::spawn(
            powershell(),
            &args(&script),
            workspace.path(),
            &environment(),
        )
        .expect("suspended spawn and assignment succeed");
        let grandchild = wait_for_pid_file(&pid_file);
        assert!(
            child.job_contains(grandchild),
            "grandchild {grandchild} spawned through Start-Process/ShellExecuteEx is not in the job"
        );
        child.terminate_job().expect("job terminates");
        child.kill_and_reap();
        assert!(
            wait_until_gone(grandchild),
            "grandchild {grandchild} survived the job termination"
        );
    }

    #[test]
    fn job_audit_lists_members_and_is_empty_after_the_kill() {
        let workspace = tempfile::tempdir().expect("workspace");
        let pid_file = workspace.path().join("audit.pid");
        let script = grandchild_first_script(&pid_file, "Start-Sleep -Seconds 120");
        let mut child = WorldProcess::spawn(
            powershell(),
            &args(&script),
            workspace.path(),
            &environment(),
        )
        .expect("suspended spawn and assignment succeed");
        let grandchild = wait_for_pid_file(&pid_file);
        let direct = child.pid();

        let audit = child.audit().expect("job membership is readable");
        assert!(
            audit.active.contains(&direct),
            "audit {audit:?} lost the direct child {direct}"
        );
        assert!(
            audit.active.contains(&grandchild),
            "audit {audit:?} lost the contained grandchild {grandchild}"
        );
        assert!(
            audit.total_created >= 2,
            "audit {audit:?} did not count the created processes"
        );

        child.terminate_job().expect("job terminates");
        child.kill_and_reap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let audit = child
                .audit()
                .expect("job membership is readable after the kill");
            if audit.active.is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the job still lists {:?} after termination",
                audit.active
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            wait_until_gone(grandchild),
            "grandchild {grandchild} survived the job termination"
        );
    }

    #[test]
    fn broker_created_process_escapes_the_job_and_the_audit() {
        let workspace = tempfile::tempdir().expect("workspace");
        let pid_file = workspace.path().join("wmi-broker.pid");
        let script = wmi_broker_script(&pid_file);
        let mut child = WorldProcess::spawn(
            powershell(),
            &args(&script),
            workspace.path(),
            &environment(),
        )
        .expect("suspended spawn and assignment succeed");
        let Some(escaped) = wait_for_pid_file_opt(&pid_file) else {
            eprintln!("WMI process creation is unavailable on this host; broker probe skipped");
            child.terminate_job().expect("job terminates");
            child.kill_and_reap();
            return;
        };

        assert!(
            !child.job_contains(escaped),
            "WMI-created process {escaped} unexpectedly joined the job; the probe no longer \
             reproduces the broker escape"
        );
        let audit = child.audit().expect("job membership is readable");
        assert!(
            !audit.active.contains(&escaped),
            "the job audit claimed the out-of-job WMI process {escaped}: {audit:?}"
        );

        child.terminate_job().expect("job terminates");
        child.kill_and_reap();
        assert!(
            process_alive(escaped),
            "the broker-created process {escaped} was expected to survive the job kill (the \
             documented escape)"
        );
        terminate_outside_process(escaped);
        assert!(
            wait_until_gone(escaped),
            "the test could not clean up the escaped WMI process {escaped}"
        );
    }

    #[test]
    fn failed_assignment_never_resumes_the_child() {
        let workspace = tempfile::tempdir().expect("workspace");
        let sentinel = workspace.path().join("child-ran.txt");
        let script = format!(
            "Set-Content -LiteralPath {} -Value 'ran'",
            ps_literal(&sentinel)
        );
        let observed = Mutex::new(None);
        // Run the real `AssignProcessToJobObject` against an invalid job handle
        // so the failure is the Win32 failure, not a simulated one.
        let result = spawn_with_assign(
            powershell(),
            &args(&script),
            workspace.path(),
            &environment(),
            &|_job, process| {
                *observed.lock().expect("pid slot") = Some(raw_process_id(process));
                assign_against_invalid_job(process)
            },
        );
        let error = match result {
            Ok(_) => panic!("assignment failure must abort the spawn"),
            Err(error) => error,
        };
        assert!(
            error.message.contains("Job Object"),
            "unexpected error: {error}"
        );
        let pid = observed
            .into_inner()
            .expect("pid slot")
            .expect("pid recorded before assignment");
        assert!(
            !sentinel.exists(),
            "the child executed even though assignment failed"
        );
        assert!(
            wait_until_gone(pid),
            "child {pid} was left running after a failed assignment"
        );
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !sentinel.exists(),
            "the child ran after the failed assignment returned"
        );
    }
}
