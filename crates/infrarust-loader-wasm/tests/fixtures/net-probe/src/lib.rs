use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};

use infrarust_plugin_sdk::prelude::*;
use wasip2::http::outgoing_handler;
use wasip2::http::types::{Fields, OutgoingRequest, Scheme};
use wasip2::io::streams::StreamError;

const LOG: &str = "probe.log";

#[derive(Default)]
struct NetProbe;

#[plugin(id = "net-probe", name = "Network Probe Fixture")]
impl Plugin for NetProbe {
    fn on_enable(&self, ctx: &Context) -> Result<(), PluginError> {
        ctx.command("probe")
            .description(
                "Runs one network or filesystem probe and appends the outcome to probe.log",
            )
            .handler(|invocation| {
                let args = invocation.args;
                let Some((tag, rest)) = args.split_first() else {
                    return;
                };
                let outcome = run(rest);
                record(tag, outcome);
            })
            .register()?;
        Ok(())
    }
}

fn record(tag: &str, outcome: Result<String, String>) {
    let line = match outcome {
        Ok(value) => format!("{tag} ok {value}\n"),
        Err(error) => format!("{tag} err {error}\n"),
    };
    if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(LOG) {
        let _ = log.write_all(line.as_bytes());
    }
}

fn run(args: &[String]) -> Result<String, String> {
    let arg = |index: usize| {
        args.get(index)
            .map(String::as_str)
            .ok_or_else(|| format!("missing argument {index}"))
    };
    match arg(0)? {
        "tcp" => tcp(arg(1)?, arg(2)?),
        "udp" => udp(arg(1)?, arg(2)?),
        "udp-bind" => udp_bind(arg(1)?),
        "udp-connect" => udp_connect(arg(1)?),
        "udp-recv" => udp_recv(arg(1)?),
        "listen" => listen(arg(1)?),
        "accept" => accept(arg(1)?),
        "dns" => dns(arg(1)?),
        "http" => http_get(arg(1)?),
        "read" => std::fs::read_to_string(arg(1)?).map_err(io_error),
        "write" => std::fs::write(arg(1)?, arg(2)?)
            .map(|()| String::new())
            .map_err(io_error),
        "append" => append(arg(1)?, arg(2)?),
        "copy" => std::fs::copy(arg(1)?, arg(2)?)
            .map(|bytes| bytes.to_string())
            .map_err(io_error),
        "truncate" => truncate(arg(1)?),
        "rename" => std::fs::rename(arg(1)?, arg(2)?)
            .map(|()| String::new())
            .map_err(io_error),
        "remove" => std::fs::remove_file(arg(1)?)
            .map(|()| String::new())
            .map_err(io_error),
        "symlink" => symlink(arg(1)?, arg(2)?),
        "fill" => fill(arg(1)?, arg(2)?.parse::<u32>().map_err(|e| e.to_string())?),
        "exists" => Ok(std::path::Path::new(arg(1)?).exists().to_string()),
        "caps" => Ok(Proxy::granted_capabilities()
            .into_iter()
            .map(Capability::to_kebab)
            .collect::<Vec<_>>()
            .join(",")),
        "trap" => panic!("net-probe was told to trap"),
        other => Err(format!("unknown probe {other}")),
    }
}

fn io_error(error: std::io::Error) -> String {
    format!("{:?}", error.kind())
}

fn append(path: &str, data: &str) -> Result<String, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(data.as_bytes()))
        .map(|()| String::new())
        .map_err(io_error)
}

fn truncate(path: &str) -> Result<String, String> {
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map(|_| String::new())
        .map_err(io_error)
}

fn symlink(target: &str, link: &str) -> Result<String, String> {
    let (mount, name) = link.split_once('/').unwrap_or(("", link));
    let guest_mount = format!("/{mount}");
    let directories = wasip2::filesystem::preopens::get_directories();
    let descriptor = directories
        .iter()
        .find(|(_, path)| path == &guest_mount || (mount.is_empty() && path == "/"))
        .map(|(descriptor, _)| descriptor)
        .ok_or_else(|| format!("no preopen for {guest_mount}"))?;
    descriptor
        .symlink_at(target, name)
        .map(|()| String::new())
        .map_err(|error| format!("{error:?}"))
}

fn fill(path: &str, megabytes: u32) -> Result<String, String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_error)?;
    let chunk = vec![0u8; 1024 * 1024];
    for _ in 0..megabytes {
        file.write_all(&chunk).map_err(io_error)?;
    }
    file.flush().map_err(io_error)?;
    Ok(format!("{megabytes}"))
}

fn tcp(addr: &str, payload: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect(addr).map_err(io_error)?;
    stream.write_all(payload.as_bytes()).map_err(io_error)?;
    let mut echoed = String::new();
    stream.read_to_string(&mut echoed).map_err(io_error)?;
    Ok(echoed)
}

fn udp(addr: &str, payload: &str) -> Result<String, String> {
    let target: SocketAddr = addr
        .to_socket_addrs()
        .map_err(io_error)?
        .next()
        .ok_or_else(|| "no address".to_owned())?;
    let local = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(local).map_err(|error| format!("bind {}", io_error(error)))?;
    let sent = socket
        .send_to(payload.as_bytes(), target)
        .map_err(io_error)?;
    Ok(sent.to_string())
}

fn udp_bind(addr: &str) -> Result<String, String> {
    let socket = UdpSocket::bind(addr).map_err(io_error)?;
    socket
        .local_addr()
        .map(|local| local.to_string())
        .map_err(io_error)
}

fn udp_connect(addr: &str) -> Result<String, String> {
    let target: SocketAddr = addr
        .to_socket_addrs()
        .map_err(io_error)?
        .next()
        .ok_or_else(|| "no address".to_owned())?;
    let local = if target.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(local).map_err(|error| format!("bind {}", io_error(error)))?;
    socket.connect(target).map_err(io_error)?;
    Ok(socket
        .local_addr()
        .map(|local| local.to_string())
        .unwrap_or_default())
}

fn udp_recv(bind: &str) -> Result<String, String> {
    let socket = UdpSocket::bind(bind).map_err(|error| format!("bind {}", io_error(error)))?;
    let local = socket
        .local_addr()
        .map(|addr| addr.to_string())
        .map_err(io_error)?;
    let _ = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open("udp.port")
        .and_then(|mut file| file.write_all(local.as_bytes()));
    let mut buf = [0u8; 1024];
    let (read, from) = socket.recv_from(&mut buf).map_err(io_error)?;
    Ok(format!(
        "{from} {}",
        String::from_utf8_lossy(&buf[..read])
    ))
}

fn listen(addr: &str) -> Result<String, String> {
    let listener = TcpListener::bind(addr).map_err(io_error)?;
    listener
        .local_addr()
        .map(|local| local.to_string())
        .map_err(io_error)
}

fn accept(addr: &str) -> Result<String, String> {
    let listener = TcpListener::bind(addr).map_err(io_error)?;
    let local = listener
        .local_addr()
        .map(|local| local.to_string())
        .map_err(io_error)?;
    let _ = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open("tcp.port")
        .and_then(|mut file| file.write_all(local.as_bytes()));
    let (mut stream, peer) = listener.accept().map_err(io_error)?;
    let _ = stream.write_all(b"accepted");
    Ok(peer.to_string())
}

fn dns(name: &str) -> Result<String, String> {
    let ips: Vec<String> = (name, 0)
        .to_socket_addrs()
        .map_err(io_error)?
        .map(|addr| addr.ip().to_string())
        .collect();
    Ok(ips.join(","))
}

fn http_get(url: &str) -> Result<String, String> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| format!("{url} is not a URL"))?;
    let (authority, path) = rest.find('/').map_or((rest, "/"), |at| rest.split_at(at));
    let scheme = match scheme {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        other => Scheme::Other(other.to_owned()),
    };
    let request = OutgoingRequest::new(Fields::new());
    request
        .set_scheme(Some(&scheme))
        .map_err(|()| "invalid scheme".to_owned())?;
    request
        .set_authority(Some(authority))
        .map_err(|()| "invalid authority".to_owned())?;
    request
        .set_path_with_query(Some(path))
        .map_err(|()| "invalid path".to_owned())?;
    let pending = outgoing_handler::handle(request, None).map_err(|code| format!("{code:?}"))?;
    pending.subscribe().block();
    let response = pending
        .get()
        .ok_or_else(|| "response not ready".to_owned())?
        .map_err(|()| "response already taken".to_owned())?
        .map_err(|code| format!("{code:?}"))?;
    let status = response.status();
    let body = response
        .consume()
        .map_err(|()| "body already taken".to_owned())?;
    let mut bytes = Vec::new();
    {
        let stream = body
            .stream()
            .map_err(|()| "body stream already taken".to_owned())?;
        loop {
            match stream.blocking_read(64 * 1024) {
                Ok(chunk) => bytes.extend(chunk),
                Err(StreamError::Closed) => break,
                Err(StreamError::LastOperationFailed(error)) => {
                    return Err(error.to_debug_string());
                }
            }
        }
    }
    Ok(format!("{status} {}", String::from_utf8_lossy(&bytes)))
}
