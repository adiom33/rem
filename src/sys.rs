// Raw libc bindings for Linux ARM (and x86_64 for dev).
// This replaces the `libc` crate so we have zero dependencies.

// ---- Fundamental types ----
pub type c_int = i32;
pub type c_uint = u32;
pub type c_long = isize;
pub type c_ulong = usize;
pub type c_short = i16;
pub type c_ushort = u16;
pub type c_char = i8;
pub type c_void = core::ffi::c_void;
pub type size_t = usize;
pub type ssize_t = isize;
pub type pid_t = i32;
pub type mode_t = u32;
pub type nfds_t = c_ulong;
pub type time_t = c_long;
pub type suseconds_t = c_long;

// ---- Constants ----

// mmap
pub const PROT_READ: c_int = 0x1;
pub const PROT_WRITE: c_int = 0x2;
pub const MAP_SHARED: c_int = 0x01;
pub const MAP_FAILED: *mut c_void = !0 as *mut c_void;

// open
pub const O_RDWR: c_int = 0x02;
pub const O_NONBLOCK: c_int = 0x800;

// fcntl
pub const F_GETFL: c_int = 3;
pub const F_SETFL: c_int = 4;

// poll
pub const POLLIN: c_short = 0x0001;
pub const POLLHUP: c_short = 0x0010;

// waitpid
pub const WNOHANG: c_int = 1;

// signals
pub const SIGHUP: c_int = 1;

// ioctl for TTY
pub const TIOCSCTTY: c_ulong = 0x540E;
pub const TIOCSWINSZ: c_ulong = 0x5414;

// framebuffer ioctls
pub const FBIOGET_VSCREENINFO: c_ulong = 0x4600;
pub const FBIOGET_FSCREENINFO: c_ulong = 0x4602;

// MXCFB_SEND_UPDATE for e-ink refresh
pub const MXCFB_SEND_UPDATE: c_ulong = 0x4048462E;

// ---- Structures ----

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct pollfd {
    pub fd: c_int,
    pub events: c_short,
    pub revents: c_short,
}

#[repr(C)]
pub struct winsize {
    pub ws_row: c_ushort,
    pub ws_col: c_ushort,
    pub ws_xpixel: c_ushort,
    pub ws_ypixel: c_ushort,
}

// ---- Extern functions ----

extern "C" {
    // File operations
    pub fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    pub fn close(fd: c_int) -> c_int;
    pub fn read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t;
    pub fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t;

    // Memory mapping
    pub fn mmap(
        addr: *mut c_void,
        len: size_t,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: isize,
    ) -> *mut c_void;
    pub fn munmap(addr: *mut c_void, len: size_t) -> c_int;

    // ioctl
    pub fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;

    // Process management
    pub fn fork() -> pid_t;
    pub fn setsid() -> pid_t;
    pub fn execvp(file: *const c_char, argv: *const *const c_char) -> c_int;
    pub fn _exit(status: c_int) -> !;
    pub fn waitpid(pid: pid_t, status: *mut c_int, options: c_int) -> pid_t;
    pub fn kill(pid: pid_t, sig: c_int) -> c_int;

    // PTY
    pub fn openpty(
        amaster: *mut c_int,
        aslave: *mut c_int,
        name: *mut c_char,
        termp: *const c_void,
        winp: *const winsize,
    ) -> c_int;

    // File descriptor operations
    pub fn dup2(oldfd: c_int, newfd: c_int) -> c_int;
    pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;

    // Poll
    pub fn poll(fds: *mut pollfd, nfds: nfds_t, timeout: c_int) -> c_int;

    // Environment
    pub fn putenv(string: *mut c_char) -> c_int;

    // Error
    pub fn __errno_location() -> *mut c_int;

    // String
    pub fn memset(s: *mut c_void, c: c_int, n: size_t) -> *mut c_void;

    // Sleep
    pub fn usleep(usec: c_uint) -> c_int;
}

/// Get the last errno value.
pub fn errno() -> c_int {
    unsafe { *__errno_location() }
}

/// Format an errno into a string.
pub fn errno_str() -> &'static str {
    match errno() {
        1 => "EPERM",
        2 => "ENOENT",
        5 => "EIO",
        9 => "EBADF",
        11 => "EAGAIN",
        12 => "ENOMEM",
        13 => "EACCES",
        14 => "EFAULT",
        19 => "ENODEV",
        22 => "EINVAL",
        25 => "ENOTTY",
        _ => "unknown",
    }
}

/// Write bytes to stderr (for debug logging without std::io dependency issues).
pub fn write_stderr(msg: &str) {
    unsafe {
        write(2, msg.as_ptr() as *const c_void, msg.len());
    }
}
