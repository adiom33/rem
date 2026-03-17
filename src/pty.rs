/// PTY (pseudo-terminal) management.
/// Forks a child process connected to a PTY.

use std::ffi::CString;
use std::io;
use std::os::unix::io::RawFd;

use crate::sys;

pub struct Pty {
    pub master_fd: RawFd,
    pub child_pid: sys::pid_t,
}

impl Pty {
    pub fn spawn(
        cols: u16,
        rows: u16,
        command: &str,
        args: &[&str],
        env_term: &str,
    ) -> Result<Self, String> {
        let mut master_fd: RawFd = -1;
        let mut slave_fd: RawFd = -1;

        let ws = sys::winsize {
            ws_col: cols,
            ws_row: rows,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        let ret = unsafe {
            sys::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &ws,
            )
        };
        if ret < 0 {
            return Err(format!("openpty failed: {} (errno {})", sys::errno_str(), sys::errno()));
        }

        let pid = unsafe { sys::fork() };
        if pid < 0 {
            return Err(format!("fork failed: {}", sys::errno_str()));
        }

        if pid == 0 {
            // ---- Child process ----
            unsafe {
                sys::close(master_fd);
                sys::setsid();
                sys::ioctl(slave_fd, sys::TIOCSCTTY, 0 as sys::c_int);
                sys::dup2(slave_fd, 0);
                sys::dup2(slave_fd, 1);
                sys::dup2(slave_fd, 2);
                if slave_fd > 2 {
                    sys::close(slave_fd);
                }

                // Set environment variables
                // These need to be leaked (not dropped) since putenv doesn't copy
                let term_env = CString::new(format!("TERM={}", env_term)).unwrap();
                sys::putenv(term_env.into_raw());

                let cols_env = CString::new(format!("COLUMNS={}", cols)).unwrap();
                sys::putenv(cols_env.into_raw());

                let rows_env = CString::new(format!("LINES={}", rows)).unwrap();
                sys::putenv(rows_env.into_raw());

                // Ensure /opt/bin is in PATH
                let cur_path = std::env::var("PATH").unwrap_or_default();
                if !cur_path.contains("/opt/bin") {
                    let path_env = CString::new(format!("PATH=/opt/bin:{}", cur_path)).unwrap();
                    sys::putenv(path_env.into_raw());
                }

                // Build argv
                let c_command = CString::new(command).unwrap();
                let c_args: Vec<CString> = args
                    .iter()
                    .map(|a| CString::new(*a).unwrap())
                    .collect();
                let mut c_argv: Vec<*const sys::c_char> =
                    c_args.iter().map(|a| a.as_ptr()).collect();
                c_argv.push(std::ptr::null());

                sys::execvp(c_command.as_ptr(), c_argv.as_ptr());

                // If execvp returns, it failed
                sys::_exit(1);
            }
        }

        // ---- Parent process ----
        unsafe {
            sys::close(slave_fd);

            // Set master to non-blocking
            let flags = sys::fcntl(master_fd, sys::F_GETFL);
            sys::fcntl(master_fd, sys::F_SETFL, flags | sys::O_NONBLOCK);
        }

        eprintln!("Spawned child PID {} running: {}", pid, command);

        Ok(Pty {
            master_fd,
            child_pid: pid,
        })
    }

    pub fn read(&self) -> Vec<u8> {
        let mut buf = [0u8; 4096];
        let n = unsafe {
            sys::read(
                self.master_fd,
                buf.as_mut_ptr() as *mut sys::c_void,
                buf.len(),
            )
        };
        if n > 0 {
            buf[..n as usize].to_vec()
        } else {
            Vec::new()
        }
    }

    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        let mut written = 0usize;
        let mut retries = 0u32;
        const MAX_RETRIES: u32 = 500; // 500 * 1ms = 500ms max wait
        while written < data.len() {
            let n = unsafe {
                sys::write(
                    self.master_fd,
                    data[written..].as_ptr() as *const sys::c_void,
                    data.len() - written,
                )
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock {
                    retries += 1;
                    if retries >= MAX_RETRIES {
                        return Err("write to PTY timed out (buffer full)".to_string());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                return Err(format!("write to PTY failed: {}", err));
            }
            if n == 0 {
                return Err("write to PTY returned 0".to_string());
            }
            written += n as usize;
            retries = 0; // Reset on successful write
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub fn set_size(&self, cols: u16, rows: u16) {
        let ws = sys::winsize {
            ws_col: cols,
            ws_row: rows,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            sys::ioctl(self.master_fd, sys::TIOCSWINSZ, &ws);
        }
    }

    pub fn is_alive(&self) -> bool {
        let mut status: sys::c_int = 0;
        let ret = unsafe { sys::waitpid(self.child_pid, &mut status, sys::WNOHANG) };
        ret == 0
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        unsafe {
            sys::close(self.master_fd);
            let mut status: sys::c_int = 0;
            let ret = sys::waitpid(self.child_pid, &mut status, sys::WNOHANG);
            if ret == 0 {
                // Child still running — send SIGHUP and wait briefly
                sys::kill(self.child_pid, sys::SIGHUP);
                // Non-blocking poll for up to 2 seconds
                for _ in 0..20 {
                    let ret = sys::waitpid(self.child_pid, &mut status, sys::WNOHANG);
                    if ret != 0 {
                        return; // Reaped
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                // Still alive after 2s — escalate to SIGKILL
                eprintln!("Child PID {} did not exit after SIGHUP, sending SIGKILL", self.child_pid);
                sys::kill(self.child_pid, 9); // SIGKILL
                sys::waitpid(self.child_pid, &mut status, 0);
            }
        }
    }
}
