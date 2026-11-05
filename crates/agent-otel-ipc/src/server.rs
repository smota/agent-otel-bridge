/* Copyright The OpenTelemetry Authors
 * SPDX-License-Identifier: Apache-2.0 */
use crate::frame::{decode_header, MsgType, HEADER_LEN};
use std::io;
use std::sync::{
    atomic::{AtomicU64, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct IngressLimits {
    pub max_frame_bytes: usize,
    pub max_connections: usize,
    pub payload_budget_bytes: usize,
    pub read_timeout: Duration,
}
impl Default for IngressLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 16 * 1024 * 1024,
            max_connections: 32,
            payload_budget_bytes: 32 * 1024 * 1024,
            read_timeout: Duration::from_millis(100),
        }
    }
}
#[derive(Default, Debug)]
pub struct IngressStats {
    pub received: AtomicU64,
    pub admitted: AtomicU64,
    pub invalid: AtomicU64,
    pub read_failed: AtomicU64,
    pub read_deadline: AtomicU64,
    pub capacity: AtomicU64,
    pub control_dropped: AtomicU64,
    pub reserved_bytes: AtomicUsize,
    pub peak_reserved_bytes: AtomicUsize,
    pub active_connections: AtomicUsize,
    pub peak_connections: AtomicUsize,

    // IPC listener lifecycle & scheduler diagnostic metrics:
    /// Count of successful CreateNamedPipe instance creations.
    pub accept_created: AtomicU64,
    /// Count of failed CreateNamedPipe instance creations.
    pub accept_create_failed: AtomicU64,
    /// Count of successful ConnectNamedPipe completions observed in main loop.
    pub accept_connected: AtomicU64,
    /// Count of failed ConnectNamedPipe attempts or panicked accept tasks.
    pub accept_connect_failed: AtomicU64,
    /// Count of accept tasks spawned via JoinSet::spawn. Does NOT mean free kernel instances.
    pub accept_spawned: AtomicU64,
    /// Count of accept task futures that reached their first poll in the runtime.
    pub accept_polled: AtomicU64,
    /// Count of replacement listener creations measured.
    pub replacement_create_count: AtomicU64,
    /// Peak duration (microseconds) of create_listener syscall.
    pub replacement_create_max_micros: AtomicU64,
    /// Count of main-loop accept receipt to replacement JoinSet::spawn calls.
    pub dispatch_to_spawn_count: AtomicU64,
    /// Peak latency (microseconds) between main loop accept receipt and replacement JoinSet::spawn.
    pub dispatch_to_spawn_max_micros: AtomicU64,
    /// Count of task connect completions received in main loop.
    pub completion_to_dispatch_count: AtomicU64,
    /// Peak latency (microseconds) between connect() returning in task and main loop join_next receipt.
    pub completion_to_dispatch_max_micros: AtomicU64,
    /// Count of spawned accept tasks reaching their first poll (same event as `accept_polled`).
    pub accept_spawn_to_poll_count: AtomicU64,
    /// Peak latency (microseconds) between JoinSet::spawn and the accept task future being first polled.
    pub accept_spawn_to_poll_max_micros: AtomicU64,
}
struct ByteLease {
    _permit: OwnedSemaphorePermit,
    bytes: usize,
    stats: Arc<IngressStats>,
}
impl Drop for ByteLease {
    fn drop(&mut self) {
        self.stats
            .reserved_bytes
            .fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
struct ConnectionLease {
    _permit: OwnedSemaphorePermit,
    stats: Arc<IngressStats>,
}
impl Drop for ConnectionLease {
    fn drop(&mut self) {
        self.stats
            .active_connections
            .fetch_sub(1, Ordering::Relaxed);
    }
}
pub struct IngressFrame {
    pub msg_type: MsgType,
    pub payload: Vec<u8>,
    pub received_at: Instant,
    _lease: ByteLease,
}
#[derive(Clone)]
enum Sink {
    Legacy(mpsc::Sender<(MsgType, Vec<u8>)>),
    Bounded {
        events: mpsc::Sender<IngressFrame>,
        control: mpsc::Sender<MsgType>,
    },
}
#[derive(Clone)]
struct ServerState {
    sink: Sink,
    shutdown: CancellationToken,
    limits: IngressLimits,
    bytes: Arc<Semaphore>,
    connections: Arc<Semaphore>,
    stats: Arc<IngressStats>,
}
impl ServerState {
    fn new(
        sink: Sink,
        shutdown: CancellationToken,
        limits: IngressLimits,
        stats: Arc<IngressStats>,
    ) -> io::Result<Self> {
        if limits.max_connections == 0
            || limits.max_connections > 4096
            || limits.max_frame_bytes == 0
            || limits.max_frame_bytes > 16 * 1024 * 1024
            || limits.payload_budget_bytes < limits.max_frame_bytes
            || limits.payload_budget_bytes > u32::MAX as usize
            || limits.read_timeout.is_zero()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid ingress limits",
            ));
        }
        Ok(Self {
            sink,
            shutdown,
            bytes: Arc::new(Semaphore::new(limits.payload_budget_bytes)),
            connections: Arc::new(Semaphore::new(limits.max_connections)),
            limits,
            stats,
        })
    }
    fn spawn_reader<R: AsyncRead + Unpin + Send + 'static>(
        &self,
        stream: R,
        readers: &mut JoinSet<()>,
    ) {
        let Ok(permit) = self.connections.clone().try_acquire_owned() else {
            self.stats.capacity.fetch_add(1, Ordering::Relaxed);
            return;
        };
        let count = self
            .stats
            .active_connections
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        self.stats
            .peak_connections
            .fetch_max(count, Ordering::Relaxed);
        let lease = ConnectionLease {
            _permit: permit,
            stats: self.stats.clone(),
        };
        let state = self.clone();
        readers.spawn(async move {
            let _lease = lease;
            tokio::select! {
                biased;
                _ = state.shutdown.cancelled() => {},
                outcome = tokio::time::timeout(state.limits.read_timeout, state.read(stream)) => {
                    match outcome {
                        Err(_) => { state.stats.read_deadline.fetch_add(1, Ordering::Relaxed); },
                        Ok(Err(_)) => { state.stats.read_failed.fetch_add(1, Ordering::Relaxed); },
                        Ok(Ok(())) => {},
                    }
                }
            }
        });
    }
    async fn read<R: AsyncRead + Unpin>(&self, mut stream: R) -> io::Result<()> {
        let mut header = [0; HEADER_LEN];
        stream.read_exact(&mut header).await?;
        let (msg_type, size) = match decode_header(&header) {
            Ok(value) => value,
            Err(_) => {
                self.stats.invalid.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        };
        let size = size as usize;
        if size > self.limits.max_frame_bytes || matches!(msg_type, MsgType::Unknown) {
            self.stats.invalid.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        let Ok(permit) = self.bytes.clone().try_acquire_many_owned(size as u32) else {
            self.stats.capacity.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        };
        let bytes = self.stats.reserved_bytes.fetch_add(size, Ordering::Relaxed) + size;
        self.stats
            .peak_reserved_bytes
            .fetch_max(bytes, Ordering::Relaxed);
        let lease = ByteLease {
            _permit: permit,
            bytes: size,
            stats: self.stats.clone(),
        };
        let mut payload = vec![0; size];
        stream.read_exact(&mut payload).await?;
        self.stats.received.fetch_add(1, Ordering::Relaxed);
        if msg_type == MsgType::Shutdown {
            self.shutdown.cancel();
        }
        match &self.sink {
            Sink::Legacy(tx) => {
                if tx.try_send((msg_type, payload)).is_ok() {
                    self.stats.admitted.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.stats.capacity.fetch_add(1, Ordering::Relaxed);
                }
            }
            Sink::Bounded { events, control } => {
                if matches!(
                    msg_type,
                    MsgType::HookPayload | MsgType::HookPayloadWithContext
                ) {
                    let frame = IngressFrame {
                        msg_type,
                        payload,
                        received_at: Instant::now(),
                        _lease: lease,
                    };
                    if events.try_send(frame).is_ok() {
                        self.stats.admitted.fetch_add(1, Ordering::Relaxed);
                    } else {
                        self.stats.capacity.fetch_add(1, Ordering::Relaxed);
                    }
                } else if control.try_send(msg_type).is_err() {
                    self.stats.control_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        Ok(())
    }
}

/// Legacy adapter: callers own the byte budget of their tuple channel.
pub async fn run_server(
    name: Option<&str>,
    tx: mpsc::Sender<(MsgType, Vec<u8>)>,
    shutdown: CancellationToken,
) -> io::Result<()> {
    serve(
        name,
        ServerState::new(
            Sink::Legacy(tx),
            shutdown,
            IngressLimits::default(),
            Arc::default(),
        )?,
    )
    .await
}
pub async fn run_server_bounded(
    name: Option<&str>,
    events: mpsc::Sender<IngressFrame>,
    control: mpsc::Sender<MsgType>,
    shutdown: CancellationToken,
    limits: IngressLimits,
    stats: Arc<IngressStats>,
) -> io::Result<()> {
    serve(
        name,
        ServerState::new(Sink::Bounded { events, control }, shutdown, limits, stats)?,
    )
    .await
}

#[cfg(windows)]
async fn serve(name: Option<&str>, state: ServerState) -> io::Result<()> {
    use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

    #[inline]
    fn saturate_u64(micros: u128) -> u64 {
        u64::try_from(micros).unwrap_or(u64::MAX)
    }

    fn create_listener(name: &str, first: bool) -> io::Result<NamedPipeServer> {
        ServerOptions::new()
            .first_pipe_instance(first)
            .in_buffer_size(64 * 1024)
            .out_buffer_size(4096)
            .create(name)
    }

    fn arm_listener(
        listener: NamedPipeServer,
        accepts: &mut JoinSet<io::Result<(NamedPipeServer, Instant)>>,
        stats: &Arc<IngressStats>,
    ) {
        let spawn_at = Instant::now();
        stats.accept_spawned.fetch_add(1, Ordering::Relaxed);
        let stats_task = stats.clone();
        accepts.spawn(async move {
            let poll_at = Instant::now();
            stats_task.accept_polled.fetch_add(1, Ordering::Relaxed);
            let spawn_to_poll = poll_at.saturating_duration_since(spawn_at).as_micros();
            stats_task
                .accept_spawn_to_poll_count
                .fetch_add(1, Ordering::Relaxed);
            stats_task
                .accept_spawn_to_poll_max_micros
                .fetch_max(saturate_u64(spawn_to_poll), Ordering::Relaxed);

            listener.connect().await?;
            let connected_at = Instant::now();
            Ok((listener, connected_at))
        });
    }

    let name = name
        .map(str::to_owned)
        .or_else(|| std::env::var("AGENT_OTEL_PIPE").ok())
        .or_else(|| std::env::var("AGY_OTEL_PIPE").ok())
        .unwrap_or_else(|| crate::frame::DEFAULT_PIPE_NAME.into());
    let pool_size = state.limits.max_connections.clamp(1, 4);
    // Create the full pool before arming it. The first instance remains the
    // collision guard, while every pool member receives its own pending
    // ConnectNamedPipe operation below.
    let mut initial = Vec::with_capacity(pool_size);
    match create_listener(&name, true) {
        Ok(listener) => {
            state.stats.accept_created.fetch_add(1, Ordering::Relaxed);
            initial.push(listener);
        }
        Err(err) => {
            state
                .stats
                .accept_create_failed
                .fetch_add(1, Ordering::Relaxed);
            return Err(err);
        }
    }
    for _ in 1..pool_size {
        match create_listener(&name, false) {
            Ok(listener) => {
                state.stats.accept_created.fetch_add(1, Ordering::Relaxed);
                initial.push(listener);
            }
            Err(err) => {
                state
                    .stats
                    .accept_create_failed
                    .fetch_add(1, Ordering::Relaxed);
                return Err(err);
            }
        }
    }
    let mut accepts = JoinSet::new();
    for listener in initial {
        arm_listener(listener, &mut accepts, &state.stats);
    }
    let mut readers = JoinSet::new();
    loop {
        if state.shutdown.is_cancelled() {
            break;
        }
        if accepts.len() < pool_size {
            let create_start = Instant::now();
            match create_listener(&name, false) {
                Ok(listener) => {
                    state.stats.accept_created.fetch_add(1, Ordering::Relaxed);
                    let create_micros = create_start.elapsed().as_micros();
                    state
                        .stats
                        .replacement_create_count
                        .fetch_add(1, Ordering::Relaxed);
                    state
                        .stats
                        .replacement_create_max_micros
                        .fetch_max(saturate_u64(create_micros), Ordering::Relaxed);
                    arm_listener(listener, &mut accepts, &state.stats);
                }
                Err(_) => {
                    state
                        .stats
                        .accept_create_failed
                        .fetch_add(1, Ordering::Relaxed);
                    // A partial pool can still make progress. Only back off
                    // when no accept remains armed; otherwise the select below
                    // continues servicing existing listeners and readers.
                    if accepts.is_empty() {
                        tokio::select! {
                            biased;
                            _ = state.shutdown.cancelled() => break,
                            _ = readers.join_next(), if !readers.is_empty() => {},
                            _ = tokio::time::sleep(Duration::from_millis(10)) => {},
                        }
                        continue;
                    }
                }
            }
        }
        tokio::select! {
            biased;
            _ = state.shutdown.cancelled() => break,
            _ = readers.join_next(), if !readers.is_empty() => {},
            result = accepts.join_next(), if !accepts.is_empty() => {
                let join_received_at = Instant::now();
                match result {
                    Some(Ok(Ok((connected, connected_at)))) => {
                        state.stats.accept_connected.fetch_add(1, Ordering::Relaxed);
                        let comp_to_disp = join_received_at
                            .saturating_duration_since(connected_at)
                            .as_micros();
                        state.stats.completion_to_dispatch_count.fetch_add(1, Ordering::Relaxed);
                        state.stats
                            .completion_to_dispatch_max_micros
                            .fetch_max(saturate_u64(comp_to_disp), Ordering::Relaxed);

                        // Replenish the pending accept pool before dispatching the
                        // established connection to a bounded reader.
                        let dispatch_start = Instant::now();
                        let create_start = Instant::now();
                        match create_listener(&name, false) {
                            Ok(listener) => {
                                state.stats.accept_created.fetch_add(1, Ordering::Relaxed);
                                let create_micros = create_start.elapsed().as_micros();
                                state.stats.replacement_create_count.fetch_add(1, Ordering::Relaxed);
                                state.stats
                                    .replacement_create_max_micros
                                    .fetch_max(saturate_u64(create_micros), Ordering::Relaxed);

                                arm_listener(listener, &mut accepts, &state.stats);
                                let disp_to_spawn = dispatch_start.elapsed().as_micros();
                                state.stats.dispatch_to_spawn_count.fetch_add(1, Ordering::Relaxed);
                                state.stats
                                    .dispatch_to_spawn_max_micros
                                    .fetch_max(saturate_u64(disp_to_spawn), Ordering::Relaxed);
                            }
                            Err(_) => {
                                state.stats.accept_create_failed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        state.spawn_reader(connected, &mut readers);
                    }
                    Some(Ok(Err(_))) | Some(Err(_)) => {
                        state.stats.accept_connect_failed.fetch_add(1, Ordering::Relaxed);
                    }
                    None => {}
                }
            }
        }
    }
    accepts.abort_all();
    while accepts.join_next().await.is_some() {}
    readers.abort_all();
    while readers.join_next().await.is_some() {}
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux_listener {
    use std::fs::{File, Metadata, OpenOptions, TryLockError};
    use std::io;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tokio::net::{UnixListener, UnixStream};

    const CONNECT_PROBE_LIMIT: Duration = Duration::from_millis(150);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) struct SocketIdentity {
        device: u64,
        inode: u64,
        owner: u32,
    }

    pub(super) fn current_uid() -> io::Result<u32> {
        // Linux procfs reports this process's owner; no environment-supplied
        // identity, native dependency, or unsafe UID call is needed.
        Ok(std::fs::metadata("/proc/self")?.uid())
    }

    pub(super) fn owned_identity(path: &Path, owner: u32) -> io::Result<SocketIdentity> {
        let metadata = std::fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "existing Unix endpoint is not an ordinary socket",
            ));
        }
        if metadata.uid() != owner {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "existing Unix socket belongs to another user",
            ));
        }
        Ok(SocketIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner,
        })
    }

    pub(super) fn remove_if_unchanged(path: &Path, expected: SocketIdentity) -> io::Result<()> {
        if owned_identity(path, expected.owner)? != expected {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "Unix socket identity changed during startup",
            ));
        }
        std::fs::remove_file(path)
    }

    pub(super) fn lease_path(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(".lock");
        PathBuf::from(name)
    }

    pub(super) fn validate_lease_metadata(metadata: &Metadata, owner: u32) -> io::Result<()> {
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != owner
            || metadata.nlink() != 1
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Unix socket lease must be a private owned regular file without hardlinks",
            ));
        }
        Ok(())
    }

    fn lease_identity(path: &Path, owner: u32) -> io::Result<(u64, u64)> {
        let metadata = std::fs::symlink_metadata(path)?;
        validate_lease_metadata(&metadata, owner)?;
        Ok((metadata.dev(), metadata.ino()))
    }

    fn lease(path: &Path, owner: u32) -> io::Result<File> {
        let path = lease_path(path);
        let (file, expected) = match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => (file, None),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let expected = lease_identity(&path, owner)?;
                // Native processes never unlink or replace lease files. Check
                // before opening, never truncate, then verify the opened inode
                // and path again. The private run directory (or /tmp sticky
                // ownership) prevents another UID from replacing an owned file.
                let file = OpenOptions::new().read(true).write(true).open(&path)?;
                (file, Some(expected))
            }
            Err(error) => return Err(error),
        };
        let metadata = file.metadata()?;
        validate_lease_metadata(&metadata, owner)?;
        let opened = (metadata.dev(), metadata.ino());
        if expected.is_some_and(|expected| expected != opened)
            || lease_identity(&path, owner)? != opened
        {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "Unix socket lease identity changed",
            ));
        }
        file.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::WouldBlock,
                "another native listener owns the Unix socket lease",
            ),
            TryLockError::Error(error) => error,
        })?;
        if lease_identity(&path, owner)? != opened {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "Unix socket lease identity changed while locking",
            ));
        }
        Ok(file)
    }

    pub(super) struct SocketGuard {
        path: PathBuf,
        identity: SocketIdentity,
        // Drop runs before fields are destroyed: cleanup retains the lease.
        // Persistent lease files must never be unlinked; SIGKILL releases the
        // kernel lock automatically, including before any post-start hook ran.
        _lease: File,
    }
    impl Drop for SocketGuard {
        fn drop(&mut self) {
            // Permissions may legitimately change during service setup. Pin
            // the inode/owner, not its mode or ctime; never unlink a replacement.
            let _ = remove_if_unchanged(&self.path, self.identity);
        }
    }

    pub(super) async fn bind(path: &Path) -> io::Result<(UnixListener, SocketGuard)> {
        let owner = current_uid()?;
        // Refuse a pre-existing foreign/non-socket endpoint without creating
        // even an associated lease beside it.
        match std::fs::symlink_metadata(path) {
            Ok(_) => {
                owned_identity(path, owner)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let lease = lease(path, owner)?;
        let listener = match UnixListener::bind(path) {
            Ok(listener) => listener,
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                let stale = owned_identity(path, owner)?;
                // Startup only: a live listener, permission failure, timeout,
                // or any error other than ECONNREFUSED forbids takeover.
                match tokio::time::timeout(CONNECT_PROBE_LIMIT, UnixStream::connect(path)).await {
                    Ok(Err(error)) if error.kind() == io::ErrorKind::ConnectionRefused => {}
                    Ok(Err(error)) => return Err(error),
                    Ok(Ok(_peer)) => {
                        return Err(io::Error::new(
                            io::ErrorKind::AddrInUse,
                            "refusing to replace a live Unix listener",
                        ));
                    }
                    Err(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "Unix listener ownership probe timed out",
                        ));
                    }
                }
                remove_if_unchanged(path, stale)?;
                UnixListener::bind(path)?
            }
            Err(error) => return Err(error),
        };
        let identity = owned_identity(path, owner)?;
        Ok((
            listener,
            SocketGuard {
                path: path.to_owned(),
                identity,
                _lease: lease,
            },
        ))
    }
}

#[cfg(unix)]
async fn serve(name: Option<&str>, state: ServerState) -> io::Result<()> {
    let path = name
        .map(str::to_owned)
        .or_else(|| std::env::var("AGENT_OTEL_SOCKET").ok())
        .unwrap_or_else(|| "/tmp/agent_otel_bridge.sock".into());
    #[cfg(target_os = "linux")]
    let (listener, _guard) = linux_listener::bind(std::path::Path::new(&path)).await?;
    // Preserve the existing non-Linux listener lifecycle.
    #[cfg(not(target_os = "linux"))]
    let listener = tokio::net::UnixListener::bind(&path)?;
    #[cfg(not(target_os = "linux"))]
    struct SocketGuard(String);
    #[cfg(not(target_os = "linux"))]
    impl Drop for SocketGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _guard = SocketGuard(path);
    let mut readers = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _ = state.shutdown.cancelled() => break,
            _ = readers.join_next(), if !readers.is_empty() => {},
            result = listener.accept() => {
                match result { Ok((stream, _)) => state.spawn_reader(stream, &mut readers), Err(error) => return Err(error) }
            }
        }
    }
    readers.abort_all();
    while readers.join_next().await.is_some() {}
    Ok(())
}
#[cfg(not(any(windows, unix)))]
async fn serve(_name: Option<&str>, _state: ServerState) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "IPC is unsupported",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test]
    async fn byte_permits_follow_queued_frame_and_release_on_drop() {
        let (tx, mut rx) = mpsc::channel(1);
        let (ctrl, _) = mpsc::channel(1);
        let stats = Arc::new(IngressStats::default());
        let limits = IngressLimits {
            max_frame_bytes: 8,
            payload_budget_bytes: 8,
            ..IngressLimits::default()
        };
        let state = ServerState::new(
            Sink::Bounded {
                events: tx,
                control: ctrl,
            },
            CancellationToken::new(),
            limits,
            stats.clone(),
        )
        .unwrap();
        for _ in 0..2 {
            let (mut writer, reader) = tokio::io::duplex(64);
            writer
                .write_all(&crate::frame::encode_frame(
                    MsgType::HookPayload,
                    b"12345678",
                ))
                .await
                .unwrap();
            state.read(reader).await.unwrap();
        }
        assert_eq!(stats.admitted.load(Ordering::Relaxed), 1);
        assert_eq!(stats.capacity.load(Ordering::Relaxed), 1);
        assert_eq!(stats.reserved_bytes.load(Ordering::Relaxed), 8);
        drop(rx.recv().await.unwrap());
        assert_eq!(stats.reserved_bytes.load(Ordering::Relaxed), 0);
    }
    #[tokio::test]
    async fn stalled_readers_are_bounded_and_cancelled() {
        let (tx, _rx) = mpsc::channel(1);
        let stats = Arc::new(IngressStats::default());
        let state = ServerState::new(
            Sink::Legacy(tx),
            CancellationToken::new(),
            IngressLimits {
                max_connections: 1,
                read_timeout: Duration::from_millis(10),
                ..IngressLimits::default()
            },
            stats.clone(),
        )
        .unwrap();
        let (_w, r) = tokio::io::duplex(16);
        let (_w2, r2) = tokio::io::duplex(16);
        let mut set = JoinSet::new();
        state.spawn_reader(r, &mut set);
        state.spawn_reader(r2, &mut set);
        while set.join_next().await.is_some() {}
        assert_eq!(stats.peak_connections.load(Ordering::Relaxed), 1);
        assert_eq!(stats.active_connections.load(Ordering::Relaxed), 0);
        assert_eq!(stats.read_deadline.load(Ordering::Relaxed), 1);
        assert_eq!(stats.capacity.load(Ordering::Relaxed), 1);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_socket_tests {
    use super::*;
    use std::os::unix::fs::{symlink, DirBuilderExt, PermissionsExt};
    use std::os::unix::process::ExitStatusExt;
    use std::path::{Path, PathBuf};

    struct Workspace(PathBuf);
    impl Workspace {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = Path::new("/tmp").join(format!("aob-uds-{}-{nonce}", std::process::id()));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&root)
                .unwrap();
            Self(root)
        }
        fn socket(&self) -> PathBuf {
            self.0.join("socket")
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct ChildCleanup(std::process::Child);
    impl Drop for ChildCleanup {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn cancelled_state() -> ServerState {
        let (tx, _rx) = mpsc::channel(1);
        let shutdown = CancellationToken::new();
        shutdown.cancel();
        ServerState::new(
            Sink::Legacy(tx),
            shutdown,
            IngressLimits::default(),
            Arc::new(IngressStats::default()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn stale_socket_after_process_kill_is_recovered() {
        const CHILD_SOCKET: &str = "AGENT_OTEL_TEST_CRASH_SOCKET";
        // Re-execute this exact test as an owned listener subprocess. Its
        // environment is private to that child; the parent always kills/waits.
        if let Some(path) = std::env::var_os(CHILD_SOCKET) {
            let (_listener, _guard) = linux_listener::bind(Path::new(&path)).await.unwrap();
            loop {
                std::thread::park();
            }
        }
        let workspace = Workspace::new();
        let path = workspace.socket();
        let mut child = ChildCleanup(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "server::linux_socket_tests::stale_socket_after_process_kill_is_recovered",
                    "--nocapture",
                ])
                .env(CHILD_SOCKET, &path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "listener child exited early"
            );
            assert!(
                Instant::now() < deadline,
                "listener child never bound its socket"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        child.0.kill().unwrap();
        assert_eq!(child.0.wait().unwrap().signal(), Some(9));
        assert!(path.exists(), "SIGKILL must leave the stale socket fixture");
        let result = serve(Some(path.to_str().unwrap()), cancelled_state()).await;
        assert!(
            result.is_ok(),
            "owned stale listener must recover: {result:?}"
        );
        assert!(
            !path.exists(),
            "normal shutdown must remove the new owned socket"
        );
    }

    #[tokio::test]
    async fn live_listener_is_refused_without_takeover() {
        let workspace = Workspace::new();
        let path = workspace.socket();
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let before =
            linux_listener::owned_identity(&path, linux_listener::current_uid().unwrap()).unwrap();
        let error = linux_listener::bind(&path).await.err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
        assert_eq!(
            linux_listener::owned_identity(&path, linux_listener::current_uid().unwrap()).unwrap(),
            before
        );
        assert!(tokio::net::UnixStream::connect(&path).await.is_ok());
    }

    #[tokio::test]
    async fn symlink_and_non_socket_endpoints_are_preserved() {
        let workspace = Workspace::new();
        let ordinary = workspace.0.join("ordinary");
        std::fs::write(&ordinary, b"owned non-socket fixture").unwrap();
        assert!(linux_listener::bind(&ordinary).await.is_err());
        assert_eq!(
            std::fs::read(&ordinary).unwrap(),
            b"owned non-socket fixture"
        );
        let path = workspace.socket();
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let alias = workspace.0.join("alias");
        symlink(&path, &alias).unwrap();
        assert!(linux_listener::bind(&alias).await.is_err());
        assert_eq!(std::fs::read_link(&alias).unwrap(), path);
        assert!(tokio::net::UnixStream::connect(&path).await.is_ok());
    }

    #[test]
    fn socket_metadata_for_another_owner_is_refused() {
        let workspace = Workspace::new();
        let path = workspace.socket();
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let owner = linux_listener::current_uid().unwrap();
        // Real filesystem metadata, with a different prospective caller UID;
        // no privilege, chown, or foreign account's socket is needed.
        assert_eq!(
            linux_listener::owned_identity(&path, owner.wrapping_add(1))
                .unwrap_err()
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(linux_listener::owned_identity(&path, owner).is_ok());
    }

    #[tokio::test]
    async fn permission_denied_stale_socket_is_not_removed() {
        if linux_listener::current_uid().unwrap() == 0 {
            return; // Root bypasses the permission denial this fixture needs.
        }
        let workspace = Workspace::new();
        let path = workspace.socket();
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        let error = linux_listener::bind(&path).await.err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0
        );
    }

    #[tokio::test]
    async fn changed_identity_before_cleanup_preserves_replacement() {
        let workspace = Workspace::new();
        let path = workspace.socket();
        let _original = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let identity =
            linux_listener::owned_identity(&path, linux_listener::current_uid().unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        let _replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
        assert_eq!(
            linux_listener::remove_if_unchanged(&path, identity)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AddrInUse
        );
        assert!(tokio::net::UnixStream::connect(&path).await.is_ok());
    }

    #[tokio::test]
    async fn drop_guard_preserves_replacement_listener() {
        let workspace = Workspace::new();
        let path = workspace.socket();
        let (_original, guard) = linux_listener::bind(&path).await.unwrap();
        std::fs::remove_file(&path).unwrap();
        let _replacement = std::os::unix::net::UnixListener::bind(&path).unwrap();
        drop(guard);
        assert!(tokio::net::UnixStream::connect(&path).await.is_ok());
    }

    #[tokio::test]
    async fn drop_guard_cleans_owned_socket_after_service_mode_change() {
        let workspace = Workspace::new();
        let path = workspace.socket();
        let (_listener, guard) = linux_listener::bind(&path).await.unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(guard);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn saturated_live_listener_is_preserved_with_bounded_probe() {
        let workspace = Workspace::new();
        let path = workspace.socket();
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let mut connections = Vec::new();
        for _ in 0..512 {
            match tokio::time::timeout(
                Duration::from_millis(20),
                tokio::net::UnixStream::connect(&path),
            )
            .await
            {
                Ok(Ok(stream)) => connections.push(stream),
                _ => break,
            }
        }
        let identity =
            linux_listener::owned_identity(&path, linux_listener::current_uid().unwrap()).unwrap();
        let started = Instant::now();
        assert!(linux_listener::bind(&path).await.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(
            linux_listener::owned_identity(&path, linux_listener::current_uid().unwrap()).unwrap(),
            identity
        );
    }

    #[tokio::test]
    async fn native_listener_lease_spans_cleanup_and_is_reusable() {
        let workspace = Workspace::new();
        let path = workspace.socket();
        let (listener, guard) = linux_listener::bind(&path).await.unwrap();
        let identity =
            linux_listener::owned_identity(&path, linux_listener::current_uid().unwrap()).unwrap();
        let error = linux_listener::bind(&path).await.err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(
            linux_listener::owned_identity(&path, linux_listener::current_uid().unwrap()).unwrap(),
            identity
        );
        drop(listener);
        // Even after the listener closes, its cleanup guard still owns the
        // lease. Another native startup must not mistake it for abandoned state.
        let error = linux_listener::bind(&path).await.err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        drop(guard);
        assert!(!path.exists());
        assert!(linux_listener::lease_path(&path).exists());
        let (_next_listener, _next_guard) = linux_listener::bind(&path).await.unwrap();
    }

    #[tokio::test]
    async fn unsafe_lease_paths_are_refused_without_modifying_targets() {
        let workspace = Workspace::new();
        let path = workspace.socket();
        let lease = linux_listener::lease_path(&path);
        let target = workspace.0.join("preserve");
        std::fs::write(&target, b"unrelated owned fixture").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();

        symlink(&target, &lease).unwrap();
        assert!(linux_listener::bind(&path).await.is_err());
        assert!(lease.is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"unrelated owned fixture");
        std::fs::remove_file(&lease).unwrap();

        std::fs::hard_link(&target, &lease).unwrap();
        assert!(linux_listener::bind(&path).await.is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"unrelated owned fixture");
        std::fs::remove_file(&lease).unwrap();

        std::fs::create_dir(&lease).unwrap();
        assert!(linux_listener::bind(&path).await.is_err());
        assert!(lease.is_dir());
        std::fs::remove_dir(&lease).unwrap();

        std::fs::write(&lease, b"preserve private lock contents").unwrap();
        std::fs::set_permissions(&lease, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(linux_listener::bind(&path).await.is_err());
        assert_eq!(
            std::fs::read(&lease).unwrap(),
            b"preserve private lock contents"
        );
        std::fs::set_permissions(&lease, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            linux_listener::validate_lease_metadata(
                &std::fs::symlink_metadata(&lease).unwrap(),
                linux_listener::current_uid().unwrap().wrapping_add(1),
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::PermissionDenied
        );
        let (_listener, _guard) = linux_listener::bind(&path).await.unwrap();
        assert_eq!(
            std::fs::read(&lease).unwrap(),
            b"preserve private lock contents"
        );
    }
}
