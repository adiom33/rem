// Raw libc bindings for Linux ARM (and x86_64 for dev).
// This replaces the `libc` crate so we have zero dependencies.

// ---- Fundamental types ----
pub type c_int = i32;
pub type c_uint = u32;
pub type c_long = isize;
pub type c_ulong = usize;
pub type c_short = i16;
pub type c_ushort = u16;
pub type c_char = core::ffi::c_char;
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
pub const O_CREAT: c_int = 0x40;

// Socket constants (for rM2-stuff UNIX domain socket)
pub const AF_UNIX: c_int = 1;
pub const SOCK_STREAM: c_int = 1;
pub const SOL_SOCKET: c_int = 1;
pub const SO_RCVTIMEO: c_int = 20;

/// UNIX domain socket address
#[repr(C)]
pub struct sockaddr_un {
    pub sun_family: c_ushort,
    pub sun_path: [c_char; 108],
}

/// Timeval for socket timeout
#[repr(C)]
pub struct timeval {
    pub tv_sec: time_t,
    pub tv_usec: suseconds_t,
}

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
pub const SIGINT: c_int = 2;
pub const SIGTERM: c_int = 15;

// For sigaction-based signal handling
pub type sighandler_t = extern "C" fn(c_int);

// ioctl for TTY
pub const TIOCSCTTY: c_ulong = 0x540E;
pub const TIOCSWINSZ: c_ulong = 0x5414;

// framebuffer ioctls
pub const FBIOGET_VSCREENINFO: c_ulong = 0x4600;
pub const FBIOGET_FSCREENINFO: c_ulong = 0x4602;

// MXCFB_SEND_UPDATE for e-ink refresh
// V2: 72-byte struct (with dither_mode, quant_bit, alt_buffer_data)
pub const MXCFB_SEND_UPDATE: c_ulong = 0x4048462E;
// V1: 36-byte struct (older kernels / some RM2 firmware)
pub const MXCFB_SEND_UPDATE_V1: c_ulong = 0x4024462E;
// Auto-update mode: set to 1 to auto-refresh on fb writes
pub const MXCFB_SET_AUTO_UPDATE_MODE: c_ulong = 0x4004462D;
// Wait for a specific update to complete
pub const MXCFB_WAIT_FOR_UPDATE_COMPLETE: c_ulong = 0x4004462F;

// Standard Linux fb ioctls (used as RM2 SWTCON fallback)
pub const FBIOPAN_DISPLAY: c_ulong = 0x4606;
pub const FBIO_BLANK: c_ulong = 0x4611;
pub const FB_BLANK_UNBLANK: c_int = 0;

// Sys V IPC (for rm2fb message queue)
pub type key_t = c_int;
pub const IPC_NOWAIT: c_int = 0o4000;

// rm2fb message queue key
pub const RM2FB_MSG_KEY: key_t = 0x2257c;

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

    // Signal handling
    pub fn signal(signum: c_int, handler: sighandler_t) -> usize;

    // System command execution
    pub fn system(command: *const c_char) -> c_int;

    // Sys V IPC (message queues — used for rm2fb)
    pub fn msgget(key: key_t, msgflg: c_int) -> c_int;
    pub fn msgsnd(msqid: c_int, msgp: *const c_void, msgsz: size_t, msgflg: c_int) -> c_int;

    // POSIX shared memory (for rM2-stuff)
    pub fn shm_open(name: *const c_char, oflag: c_int, mode: mode_t) -> c_int;
    pub fn ftruncate(fd: c_int, length: isize) -> c_int;

    // Sockets (for rM2-stuff UNIX domain socket)
    pub fn socket(domain: c_int, sock_type: c_int, protocol: c_int) -> c_int;
    pub fn connect(sockfd: c_int, addr: *const c_void, addrlen: c_uint) -> c_int;
    pub fn setsockopt(sockfd: c_int, level: c_int, optname: c_int,
                      optval: *const c_void, optlen: c_uint) -> c_int;
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
