/* Linux x86_64 production hook only: panic=abort, no exception unwinding,
 * pthread_exit, or pthread_cancel. Rust diagnostic backtraces may truncate.
 * Keep .gcc_except_table and let the linker preserve RELRO and program headers.
 * Never apply this script to Cargo's unwinding test harnesses or the daemon. */
SECTIONS {
  /DISCARD/ : { *(.eh_frame .eh_frame.* .eh_frame_hdr) }
}
INSERT AFTER .rodata;
