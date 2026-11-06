//! Mapping and unmapping whole pages. Unmapping is the only way to be sure memory leaves the
//! process, whatever the allocator's policy.

/// Whether pages are mapped here on this target. Where they are not, every block goes to the system
/// allocator and nothing in this module is reached.
///
/// The foreign declarations below are written by hand, so they are right only where they were
/// checked. `MAP_ANON` is `0x1000` on Apple and the BSDs, `0x20` on most of Linux and `0x800` on
/// Linux MIPS. `mmap`'s offset is 64 bits only where `off_t` is. Wrong flags fail every large
/// allocation and a wrong offset type is undefined behavior, so the list is explicit: Apple,
/// 64-bit Linux and Android on the architectures named, and Windows.
pub(crate) const SUPPORTED: bool = cfg!(any(
    windows,
    target_vendor = "apple",
    all(
        any(target_os = "linux", target_os = "android"),
        target_pointer_width = "64",
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64",
            target_arch = "powerpc64",
            target_arch = "s390x",
            target_arch = "loongarch64",
        ),
    ),
));

/// Maps `len` bytes of zeroed, readable and writable memory, aligned to at least a page (4 KiB on
/// every platform this runs on). Returns null if the system refuses.
pub(crate) fn map(len: usize) -> *mut u8 {
    os::map(len)
}

/// Unmaps a block from [`map`], at the length it was mapped with.
///
/// # Safety
///
/// `ptr` and `len` are exactly a mapping [`map`] made, and nothing uses it afterwards.
pub(crate) unsafe fn unmap(ptr: *mut u8, len: usize) {
    // SAFETY: the caller's contract.
    unsafe { os::unmap(ptr, len) }
}

#[cfg(any(
    target_vendor = "apple",
    all(
        any(target_os = "linux", target_os = "android"),
        target_pointer_width = "64",
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64",
            target_arch = "powerpc64",
            target_arch = "s390x",
            target_arch = "loongarch64",
        ),
    ),
))]
mod os {
    use core::ffi::c_void;

    const PROT_READ: i32 = 1;
    const PROT_WRITE: i32 = 2;
    const MAP_PRIVATE: i32 = 2;
    #[cfg(target_vendor = "apple")]
    const MAP_ANON: i32 = 0x1000;
    #[cfg(not(target_vendor = "apple"))]
    const MAP_ANON: i32 = 0x20;

    unsafe extern "C" {
        fn mmap(
            addr: *mut c_void,
            len: usize,
            prot: i32,
            flags: i32,
            fd: i32,
            offset: i64,
        ) -> *mut c_void;
        fn munmap(addr: *mut c_void, len: usize) -> i32;
    }

    pub(super) fn map(len: usize) -> *mut u8 {
        // SAFETY: an anonymous private mapping at an address the kernel chooses touches none of
        // this program's memory. `MAP_FAILED` is `-1`, turned into null below.
        let ptr = unsafe {
            mmap(
                core::ptr::null_mut(),
                len,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANON,
                -1,
                0,
            )
        };
        if ptr as isize == -1 {
            core::ptr::null_mut()
        } else {
            ptr.cast()
        }
    }

    pub(super) unsafe fn unmap(ptr: *mut u8, len: usize) {
        // SAFETY: the caller hands back exactly a mapping `map` made.
        let _ = unsafe { munmap(ptr.cast(), len) };
    }
}

#[cfg(windows)]
mod os {
    use core::ffi::c_void;

    const MEM_COMMIT: u32 = 0x1000;
    const MEM_RESERVE: u32 = 0x2000;
    const MEM_RELEASE: u32 = 0x8000;
    const PAGE_READWRITE: u32 = 0x04;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
        fn VirtualFree(addr: *mut c_void, size: usize, kind: u32) -> i32;
    }

    pub(super) fn map(len: usize) -> *mut u8 {
        // SAFETY: a fresh committed region at an address of the system's choosing.
        unsafe {
            VirtualAlloc(
                core::ptr::null_mut(),
                len,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
        }
        .cast()
    }

    pub(super) unsafe fn unmap(ptr: *mut u8, _len: usize) {
        // SAFETY: the caller hands back exactly a region `map` made. `MEM_RELEASE` takes size zero.
        let _ = unsafe { VirtualFree(ptr.cast(), 0, MEM_RELEASE) };
    }
}

/// Everywhere else: never reached, since [`SUPPORTED`] keeps every block with the system allocator.
#[cfg(not(any(
    windows,
    target_vendor = "apple",
    all(
        any(target_os = "linux", target_os = "android"),
        target_pointer_width = "64",
        any(
            target_arch = "x86_64",
            target_arch = "aarch64",
            target_arch = "riscv64",
            target_arch = "powerpc64",
            target_arch = "s390x",
            target_arch = "loongarch64",
        ),
    ),
)))]
mod os {
    pub(super) fn map(_len: usize) -> *mut u8 {
        core::ptr::null_mut()
    }

    pub(super) unsafe fn unmap(_ptr: *mut u8, _len: usize) {}
}
