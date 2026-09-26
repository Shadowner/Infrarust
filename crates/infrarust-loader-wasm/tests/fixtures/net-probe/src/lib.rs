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
        "listen" => listen(arg(1)?),
        "dns" => dns(arg(1)?),
        "http" => http_get(arg(1)?),
        "read" => std::fs::read_to_string(arg(1)?).map_err(io_error),
        "write" => std::fs::write(arg(1)?, arg(2)?)
            .map(|()| String::new())
            .map_err(io_error),
        "exists" => Ok(std::path::Path::new(arg(1)?).exists().to_string()),
        "caps" => Ok(Proxy::granted_capabilities()
            .into_iter()
            .map(Capability::as_str)
            .collect::<Vec<_>>()
            .join(",")),
        "trap" => panic!("net-probe was told to trap"),
        other => Err(format!("unknown probe {other}")),
    }
}

fn io_error(error: std::io::Error) -> String {
    format!("{:?}", error.kind())
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

fn listen(addr: &str) -> Result<String, String> {
    let listener = TcpListener::bind(addr).map_err(io_error)?;
    listener
        .local_addr()
        .map(|local| local.to_string())
        .map_err(io_error)
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
