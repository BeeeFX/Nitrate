//! Keeping helper processes from outliving the reason they were started.
//!
//! Windows doesn't end a process's children when the process ends. Quitting
//! from the tray, the updater's exit, or a crash all left ffmpeg encoding into
//! the Downloads folder and yt-dlp downloading in the background, with no
//! window left to say so — and a running `ffmpeg.exe` is also a locked file,
//! which is precisely what the updater's installer then has to overwrite.
//!
//! Cancelling had the same gap one level down. `Child::kill` stops only the
//! process it names, and yt-dlp does its merging in an ffmpeg of its own — so
//! a cancelled download kept that running, holding the file the cleanup was
//! trying to delete.
//!
//! So every helper is started inside a job object of its own, set to kill
//! everything in it when the job's last handle closes. That handle lives in
//! `Contained`: dropping it after the process has finished costs nothing,
//! `kill` terminates the whole tree, and when the app goes away, however it
//! goes, the OS closes the handle and the tree goes with it.
//!
//! One job per helper rather than one for the app itself, deliberately. Our
//! own process in a kill-on-close job would take everything it launched down
//! with it — the update installer, which has to outlive us by design, and
//! whatever player "Play" opened.
//!
//! Elsewhere this is plain process spawning: the shipped build is Windows,
//! and there the problem is real.

use std::io;
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command, ExitStatus, Output, Stdio};

/// A running helper, and the job that ties its process tree to us.
pub struct Contained {
    child: Child,
    #[cfg(windows)]
    job: Option<job::Job>,
}

impl Contained {
    /// Stops the process and everything it started.
    pub fn kill(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
        self.child.kill()
    }

    /// `Child::wait_with_output`, keeping the job until the process is done.
    pub fn wait_with_output(self) -> io::Result<Output> {
        let Contained {
            child,
            #[cfg(windows)]
            job,
        } = self;
        let output = child.wait_with_output();
        #[cfg(windows)]
        drop(job);
        output
    }
}

impl Deref for Contained {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}

impl DerefMut for Contained {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

/// `spawn`, `output` and `status`, with the process contained.
pub trait Spawn {
    fn spawn_contained(&mut self) -> io::Result<Contained>;

    /// As `Command::output`: stdin closed, stdout and stderr captured.
    fn output_contained(&mut self) -> io::Result<Output>;

    fn status_contained(&mut self) -> io::Result<ExitStatus>;
}

impl Spawn for Command {
    fn spawn_contained(&mut self) -> io::Result<Contained> {
        let child = self.spawn()?;
        #[cfg(windows)]
        let job = job::Job::holding(&child);
        Ok(Contained {
            child,
            #[cfg(windows)]
            job,
        })
    }

    fn output_contained(&mut self) -> io::Result<Output> {
        self.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn_contained()?
            .wait_with_output()
    }

    fn status_contained(&mut self) -> io::Result<ExitStatus> {
        let mut child = self.spawn_contained()?;
        child.wait()
    }
}

#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct Job(HANDLE);

    // The handle is an owned kernel object, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// A kill-on-close job holding `child`, or `None` if Windows said no.
        ///
        /// Failure isn't fatal: the helper still runs, it just isn't tied to
        /// us, which is exactly how it ran before any of this.
        ///
        /// There's a moment between the process starting and joining the job.
        /// Anything it starts in that moment escapes — but ffmpeg starts
        /// nothing, and yt-dlp's own children come much later, after it has
        /// unpacked itself and fetched a page.
        pub fn holding(child: &Child) -> Option<Job> {
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return None;
                }
                let job = Job(handle);

                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let limited = SetInformationJobObject(
                    job.0,
                    JobObjectExtendedLimitInformation,
                    &limits as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if limited == 0 {
                    return None;
                }

                if AssignProcessToJobObject(job.0, child.as_raw_handle() as HANDLE) == 0 {
                    return None;
                }
                Some(job)
            }
        }

        pub fn terminate(&self) {
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}
