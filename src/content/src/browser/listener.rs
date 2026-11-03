use std::io;

#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};

#[cfg(windows)]
pub(super) type Connection = NamedPipeServer;
#[cfg(unix)]
pub(super) type Connection = UnixStream;

pub(super) struct Listener {
    #[cfg(windows)]
    pipe: NamedPipeServer,
    #[cfg(windows)]
    name: String,
    #[cfg(unix)]
    socket: UnixListener,
}

impl Listener {
    pub(super) fn bind(name: &str) -> io::Result<Self> {
        #[cfg(windows)]
        {
            Ok(Self {
                pipe: super::security::create_pipe(name, true)?,
                name: name.into(),
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let socket = UnixListener::bind(name)?;
            std::fs::set_permissions(name, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self { socket })
        }
    }

    pub(super) async fn accept(&mut self) -> io::Result<Connection> {
        #[cfg(windows)]
        {
            self.pipe.connect().await?;
            let next = super::security::create_pipe(&self.name, false)?;
            Ok(std::mem::replace(&mut self.pipe, next))
        }
        #[cfg(unix)]
        loop {
            let (connection, _) = self.socket.accept().await?;
            // Private permissions and peer credentials both enforce current-user access.
            if connection.peer_cred()?.uid() == unsafe { libc::geteuid() } {
                return Ok(connection);
            }
        }
    }
}
