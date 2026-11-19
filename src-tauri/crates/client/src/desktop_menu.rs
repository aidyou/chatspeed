//! Same-user desktop menu transport. This is separate from the runtime HTTP
//! API: it carries only the fixed commands that can appear in the Vue menu.

use std::{future::Future, io, path::Path, time::Duration};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const VERSION: u8 = 1;
const ACCEPTED: u8 = 1;
const REJECTED: u8 = 0;
const TIMEOUT: Duration = Duration::from_secs(2);

/// IDs are part of the versioned wire format; do not reorder or reuse them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Action {
    Chat = 1,
    Assistant = 2,
    Workflow = 3,
    Note = 4,
    ProxySwitcher = 5,
    Settings = 6,
    Model = 7,
    Agent = 8,
    Proxy = 9,
    ChatHub = 10,
    Mcp = 11,
    Skill = 12,
    ScraperTest = 13,
    About = 14,
    Quit = 15,
}

const ACTIONS: [Action; 15] = [
    Action::Chat,
    Action::Assistant,
    Action::Workflow,
    Action::Note,
    Action::ProxySwitcher,
    Action::Settings,
    Action::Model,
    Action::Agent,
    Action::Proxy,
    Action::ChatHub,
    Action::Mcp,
    Action::Skill,
    Action::ScraperTest,
    Action::About,
    Action::Quit,
];

/// Sends one request to the desktop for this runtime profile, without starting it.
pub async fn request(runtime_dir: &Path, action: Action) -> io::Result<()> {
    tokio::time::timeout(TIMEOUT, async {
        let mut stream = platform::connect(runtime_dir).await?;
        stream.write_all(&[VERSION, action as u8]).await?;
        if stream.read_u8().await? == ACCEPTED {
            Ok(())
        } else {
            Err(io::Error::other("desktop menu request rejected"))
        }
    })
    .await?
}

/// Serves one request at a time. Neither a slow peer nor a slow handler can
/// indefinitely block subsequent requests.
pub async fn serve<F, Fut>(runtime_dir: &Path, dispatch: F) -> io::Result<()>
where
    F: Fn(Action) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut listener = platform::Listener::bind(runtime_dir)?;
    loop {
        let mut stream = listener.accept().await?;
        let _ = tokio::time::timeout(TIMEOUT, exchange(&mut stream, &dispatch)).await;
    }
}

async fn exchange<F, Fut>(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    dispatch: &F,
) -> io::Result<()>
where
    F: Fn(Action) -> Fut,
    Fut: Future<Output = bool>,
{
    let mut frame = [0; 2];
    stream.read_exact(&mut frame).await?;
    let action = ACTIONS
        .iter()
        .copied()
        .find(|action| *action as u8 == frame[1]);
    let accepted = match (frame[0], action) {
        (VERSION, Some(action)) => dispatch(action).await,
        _ => false,
    };
    stream
        .write_u8(if accepted { ACCEPTED } else { REJECTED })
        .await?;
    // The named-pipe server may discard unread data when it closes. Keep it
    // open until the client has read the reply and closed, within the deadline.
    let _ = stream.read_u8().await;
    Ok(())
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
        path::PathBuf,
    };
    use tokio::net::{UnixListener, UnixStream};

    const SOCKET: &str = "desktop-menu-v1.sock";

    fn user_id() -> u32 {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() }
    }

    pub async fn connect(dir: &Path) -> io::Result<UnixStream> {
        let stream = UnixStream::connect(dir.join(SOCKET)).await?;
        if stream.peer_cred()?.uid() != user_id() {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(stream)
    }

    pub struct Listener {
        listener: UnixListener,
        path: PathBuf,
        _lock: fs::File,
    }

    impl Listener {
        pub fn bind(dir: &Path) -> io::Result<Self> {
            // The runtime already provides an owner-only directory. Never publish
            // an endpoint or remove a stale socket in an untrusted directory.
            let metadata = fs::symlink_metadata(dir)?;
            if !metadata.is_dir() || metadata.uid() != user_id() || metadata.mode() & 0o077 != 0 {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            let lock = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(dir.join("desktop-menu-v1.lock"))?;
            if !lock.metadata()?.is_file() {
                return Err(io::ErrorKind::InvalidData.into());
            }
            lock.try_lock().map_err(|error| match error {
                fs::TryLockError::WouldBlock => io::ErrorKind::AddrInUse.into(),
                fs::TryLockError::Error(error) => error,
            })?;
            let path = dir.join(SOCKET);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_socket() && meta.uid() == user_id() => {
                    fs::remove_file(&path)?
                }
                Ok(_) => return Err(io::ErrorKind::AlreadyExists.into()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            Ok(Self {
                listener: UnixListener::bind(&path)?,
                path,
                _lock: lock,
            })
        }

        pub async fn accept(&mut self) -> io::Result<UnixStream> {
            loop {
                let (stream, _) = self.listener.accept().await?;
                if stream.peer_cred().is_ok_and(|peer| peer.uid() == user_id()) {
                    return Ok(stream);
                }
            }
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        hash::{DefaultHasher, Hash, Hasher},
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        ptr,
    };
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };
    use windows_sys::Win32::{
        Foundation::{LocalFree, HANDLE},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SDDL_REVISION_1,
            },
            GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
            TOKEN_USER,
        },
        System::{
            Pipes::GetNamedPipeServerProcessId,
            Threading::{
                GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    };

    fn process_sid(process: HANDLE) -> io::Result<String> {
        // SAFETY: output pointers are valid; the token and SID string are owned
        // only after successful calls and released by their matching APIs.
        unsafe {
            let mut token: HANDLE = ptr::null_mut();
            if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
                return Err(io::Error::last_os_error());
            }
            let _token = OwnedHandle::from_raw_handle(token);
            let mut bytes = 0;
            GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut bytes);
            let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
            if bytes == 0
                || GetTokenInformation(
                    token,
                    TokenUser,
                    buffer.as_mut_ptr().cast(),
                    bytes,
                    &mut bytes,
                ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            let mut sid = ptr::null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
                return Err(io::Error::last_os_error());
            }
            let len = (0..).take_while(|i| *sid.add(*i) != 0).count();
            let value = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
            LocalFree(sid.cast());
            Ok(value)
        }
    }

    fn pipe_name(dir: &Path, sid: &str) -> io::Result<String> {
        let mut hash = DefaultHasher::new();
        dir.canonicalize()?
            .to_string_lossy()
            .to_lowercase()
            .hash(&mut hash);
        Ok(format!(
            r"\\.\pipe\ChatSpeed-desktop-menu-v1-{sid}-{:016x}",
            hash.finish()
        ))
    }

    fn current_sid() -> io::Result<String> {
        // SAFETY: the current process pseudo-handle is always valid.
        process_sid(unsafe { GetCurrentProcess() })
    }

    pub async fn connect(dir: &Path) -> io::Result<NamedPipeClient> {
        let sid = current_sid()?;
        let client = ClientOptions::new().open(pipe_name(dir, &sid)?)?;
        // A predictable name alone does not prove the identity of its server.
        // SAFETY: the pipe is open and the process handle is owned on success.
        let server_sid = unsafe {
            let mut pid = 0;
            if GetNamedPipeServerProcessId(client.as_raw_handle(), &mut pid) == 0 {
                return Err(io::Error::last_os_error());
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return Err(io::Error::last_os_error());
            }
            let _process = OwnedHandle::from_raw_handle(process);
            process_sid(process)?
        };
        if server_sid != sid {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        Ok(client)
    }

    fn create_pipe(name: &str, sid: &str, first: bool) -> io::Result<NamedPipeServer> {
        let sddl: Vec<u16> = format!("D:P(A;;GA;;;{sid})")
            .encode_utf16()
            .chain([0])
            .collect();
        // SAFETY: Windows returns an allocated descriptor. CreateNamedPipe uses
        // it synchronously and does not borrow it after the call.
        unsafe {
            let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut attrs = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let pipe = ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    name,
                    (&mut attrs as *mut SECURITY_ATTRIBUTES).cast(),
                );
            LocalFree(descriptor);
            pipe
        }
    }

    pub struct Listener {
        name: String,
        sid: String,
        pipe: NamedPipeServer,
    }

    impl Listener {
        pub fn bind(dir: &Path) -> io::Result<Self> {
            let sid = current_sid()?;
            let name = pipe_name(dir, &sid)?;
            let pipe = create_pipe(&name, &sid, true)?;
            Ok(Self { name, sid, pipe })
        }

        pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
            self.pipe.connect().await?;
            let next = create_pipe(&self.name, &self.sid, false)?;
            Ok(std::mem::replace(&mut self.pipe, next))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn menu_exchange_rejects_unknown_actions_and_versions_without_dispatch() {
        for (frame, reply) in [
            ([VERSION, 15], ACCEPTED),
            ([VERSION, 99], REJECTED),
            ([99, 15], REJECTED),
        ] {
            let (mut client, mut server) = tokio::io::duplex(8);
            let called = std::cell::Cell::new(false);
            let dispatch = |action| {
                assert_eq!(action, Action::Quit);
                called.set(true);
                std::future::ready(true)
            };
            let (server, ()) = tokio::join!(exchange(&mut server, &dispatch), async move {
                client.write_all(&frame).await.expect("request");
                assert_eq!(client.read_u8().await.expect("reply"), reply);
            });
            server.expect("exchange");
            assert_eq!(called.get(), reply == ACCEPTED);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_endpoint_is_profile_scoped_and_releases_on_shutdown() {
        let dir = tempfile::Builder::new()
            .permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .tempdir()
            .expect("profile");
        assert!(request(dir.path(), Action::Chat).await.is_err());
        let listener = platform::Listener::bind(dir.path()).expect("listen");
        assert!(platform::Listener::bind(dir.path()).is_err());
        drop(listener);
        let location = dir.path().to_owned();
        let server = tokio::spawn(async move {
            serve(&location, |action| {
                std::future::ready(action == Action::Chat)
            })
            .await
        });
        tokio::task::yield_now().await;
        request(dir.path(), Action::Chat).await.expect("accepted");
        assert!(request(dir.path(), Action::Quit).await.is_err());
        server.abort();
        let _ = server.await;
        assert!(request(dir.path(), Action::Chat).await.is_err());
    }
}
