//! Windows has no `getifaddrs` or `if_nameindex`; `GetAdaptersAddresses` is
//! the one enumeration, read the way C++ `C4NetIO::GetLocalAddresses` reads it
//! (pinned oracle `src/C4NetIO.cpp:278-305`).

use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
    GAA_FLAG_SKIP_FRIENDLY_NAME, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::Networking::WinSock::ADDRESS_FAMILY;

/// Asks only for what callers read: the interface indices and the unicast
/// address list. These are the flags C++ passes.
const FLAGS: u32 = GAA_FLAG_SKIP_ANYCAST
    | GAA_FLAG_SKIP_MULTICAST
    | GAA_FLAG_SKIP_DNS_SERVER
    | GAA_FLAG_SKIP_FRIENDLY_NAME;
const ERROR_BUFFER_OVERFLOW: u32 = 111;
/// The documented retry shape: size the buffer, then fill it. The table can
/// grow between the two calls, so this retries rather than trusting the first
/// answer, and gives up rather than looping forever.
const ATTEMPTS: usize = 3;

/// Calls `visit` on every adapter Windows reports for `family`, in table
/// order. Nothing is visited when the table cannot be read.
///
/// Every pointer reachable from a visited adapter, such as its unicast address
/// list, points into the same table and stays valid for the duration of that
/// call.
pub(crate) fn for_each_adapter(
    family: ADDRESS_FAMILY,
    mut visit: impl FnMut(&IP_ADAPTER_ADDRESSES_LH),
) {
    let mut size: u32 = 0;
    // `u64` words keep the table at the 8-byte alignment its nodes need.
    let mut buffer: Vec<u64> = Vec::new();
    for _ in 0..ATTEMPTS {
        // SAFETY: a null buffer with `size == 0` is the documented way to ask
        // for the required length; Windows writes it through `size` and returns
        // ERROR_BUFFER_OVERFLOW without touching the buffer.
        let needed = unsafe {
            GetAdaptersAddresses(
                u32::from(family),
                FLAGS,
                std::ptr::null(),
                std::ptr::null_mut(),
                &mut size,
            )
        };
        // ERROR_BUFFER_OVERFLOW is the expected answer to the sizing call;
        // anything else means there is nothing to enumerate.
        if needed != ERROR_BUFFER_OVERFLOW || size == 0 {
            return;
        }
        buffer.clear();
        buffer.resize((size as usize).div_ceil(size_of::<u64>()), 0);
        // SAFETY: `buffer` holds at least `size` bytes and outlives the walk
        // below. Windows fills it with a linked list whose `Next` chain
        // terminates at null, and every node is inside the buffer it just
        // sized.
        let result = unsafe {
            GetAdaptersAddresses(
                u32::from(family),
                FLAGS,
                std::ptr::null(),
                buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>(),
                &mut size,
            )
        };
        if result == 0 {
            let mut adapter = buffer.as_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
            // SAFETY: walking the chain Windows just wrote, stopping at its
            // null terminator.
            while let Some(current) = unsafe { adapter.as_ref() } {
                visit(current);
                adapter = current.Next;
            }
            return;
        }
        // ERROR_BUFFER_OVERFLOW again: the table grew, so size and retry.
        if result != ERROR_BUFFER_OVERFLOW {
            return;
        }
    }
}
