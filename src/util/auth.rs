use rand::Rng;
use std::{fs, io, net, path};

use crate::error::Error;

const GEN_CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const GEN_LENGTH: usize = 25;
const LT: &str = "bwt::auth";

pub enum AuthMethod {
    UserProvided(String),
    Cookie(path::PathBuf),
    Ephemeral,
    None,
}

impl AuthMethod {
    pub fn get_token(self) -> Result<Option<String>, io::Error> {
        Ok(match self {
            AuthMethod::UserProvided(token) => Some(token),
            AuthMethod::Cookie(file) => Some(read_write_cookie(&file)?),
            AuthMethod::Ephemeral => Some(generate_token()),
            AuthMethod::None => None,
        })
    }
}

fn generate_token() -> String {
    let mut rng = rand::thread_rng();
    (0..GEN_LENGTH)
        .map(|_| {
            let idx = rng.gen_range(0..GEN_CHARSET.len());
            GEN_CHARSET[idx] as char
        })
        .collect()
}

fn read_write_cookie(file: &path::Path) -> Result<String, io::Error> {
    if file.exists() {
        info!(
            target: LT,
            "Reading from cookie file: {}",
            file.to_string_lossy()
        );
        fs::read_to_string(file)
    } else {
        info!(
            target: LT,
            "Writing generated token to cookie file: {}",
            file.to_string_lossy()
        );
        let token = generate_token();
        fs::write(file, token.clone())?;
        Ok(token)
    }
}

/// Authenticate Electrum clients, using either the SOCKS5 hack or a JSON message
#[cfg(feature = "electrum")]
pub fn electrum_auth(
    mut stream: net::TcpStream,
    access_token: &str,
) -> Result<net::TcpStream, Error> {
    use std::io::{BufRead, BufReader, Read, Write};

    const SOCKS5: u8 = 0x05;
    const JSON_LB: u8 = 0x7b; // the '{' character

    let read_byte = |reader: &mut BufReader<_>| -> io::Result<_> {
        let mut buf = [0; 1];
        reader.read_exact(&mut buf)?;
        Ok(buf[0])
    };

    // Read a variable-length value prefixed by its 1-byte length
    let read_var = |reader: &mut BufReader<_>| -> Result<_, Error> {
        let len = read_byte(reader)? as u64;
        let mut buf = vec![];
        let mut chunk = reader.take(len);
        chunk.read_to_end(&mut buf)?;
        ensure!(buf.len() as u64 == len, "unexpected EOF");
        Ok(buf)
    };

    // SOCKS5 authentication hack for the Electrum RPC server
    //
    // The Electrum protocol does not natively support authentication, but some wallets supports SOCKS5.
    // To enable authentication for these, we masquerade as a SOCKS5 proxy server, parse the protocol
    // handshake and require that the SOCKS5 authentication password matches the token. The connection
    // is then handed over to real Electrum RPC server and continues as usual.
    //
    // For clients that support setting a SOCKS5 proxy but not a password for it, the token can also
    // be provided as the destination hostname.
    let socks5_auth =
        |stream: &mut net::TcpStream, reader: &mut BufReader<_>| -> Result<(), Error> {
            const AUTH_VER: u8 = 0x01;
            const AUTH_NONE: u8 = 0x00;
            const AUTH_USERPWD: u8 = 0x02;
            const SUCCESS: u8 = 0x00;
            const CONNECT: u8 = 0x01;
            const RSV: u8 = 0x00;
            const ADDR_IPV4: u8 = 0x01;
            const ADDR_DOMAIN: u8 = 0x03;

            // Client greeting: VER=0x05, <AUTH_LEN><AUTH_METHODS>
            // (the VER was already consumed to check the auth method type below)
            let auth_methods = read_var(reader)?;
            let mut authenticated = false;

            if auth_methods.contains(&AUTH_USERPWD) {
                stream.write_all(&[SOCKS5, AUTH_USERPWD])?;

                // Client authentication: VER=0x01, <USERLEN><USER>, <PWDLEN><PWD>
                ensure!(read_byte(reader)? == AUTH_VER, "invalid auth version");
                let _username = read_var(reader)?; // the username can be anything
                let password = String::from_utf8(read_var(reader)?)?;
                ensure!(password == access_token, "invalid token (userpwd)");
                authenticated = true;
                stream.write_all(&[AUTH_VER, SUCCESS])?;
            } else if auth_methods.contains(&AUTH_NONE) {
                // Allow no authentication for now, require hostname-based authentication instead (below)
                stream.write_all(&[SOCKS5, AUTH_NONE])?;
            } else {
                bail!("incompatible auth methods");
            }

            // Client connection request: VER=0x05, CMD=CONNECT(0x01), RSV=0x00, DSTADDR, DSTPORT(2 bytes)
            // Where DSTADDR is either {IPv4(0x01), <ADDR>(4 bytes)} or {DOMAIN(0x03), <ADDRLEN><ADDR>}
            let mut buf = [0; 4];
            reader.read_exact(&mut buf)?;
            ensure!(buf[0..3] == [SOCKS5, CONNECT, RSV], "invalid connect");
            match buf[3] {
                // Consume and ignore IPv4 addresses
                ADDR_IPV4 => reader.read_exact(&mut [0; 4])?,
                // Check for alternative authentication method, using the hostname as the access token
                ADDR_DOMAIN => {
                    let hostname = String::from_utf8(read_var(reader)?)?;
                    ensure!(authenticated || hostname == access_token, "invalid token");
                    authenticated = true;
                }
                _ => bail!("invalid socks5 address type"),
            };
            ensure!(authenticated, "no token was offered");

            // Consume and ignore the DSTPORT
            reader.read_exact(&mut [0; 2])?;

            // Server response: VER, STATUS, RSV, BNDADDR={IPv4(0x01), =0.0.0.0}, BNDPORT=0x0000
            stream.write_all(&[
                SOCKS5, SUCCESS, RSV, ADDR_IPV4, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ])?;

            Ok(())
        };

    // JSON based authentication, using a message formatted as:
    // { "method": "personal.auth", "params": [ "mySecretAccessToken" ] }
    // No wallets actually support this, but perhaps some will some day.
    let json_auth = |stream: &mut net::TcpStream, reader: &mut BufReader<_>| -> Result<(), Error> {
        let mut line = Vec::<u8>::new();
        reader.read_until(b'\n', &mut line)?;
        line.insert(0, JSON_LB);
        let req: serde_json::Value = serde_json::from_str(&String::from_utf8(line)?)?;
        if req["method"].as_str() == Some("personal.auth") {
            ensure!(
                req["params"][0].as_str() == Some(access_token),
                "invalid token"
            );
            let reply =
                json!({ "id": req["id"], "jsonrpc": "2.0", "result": true }).to_string() + "\n";
            stream.write_all(reply.as_bytes())?;
            Ok(())
        } else {
            bail!("invalid request, authentication expected: {}", req);
        }
    };

    let mut reader = BufReader::new(stream.try_clone().expect("failed to clone TcpStream"));

    // Read the first byte of the request to determine whether its SOCKS5 or JSON authentication
    match read_byte(&mut reader)? {
        SOCKS5 => socks5_auth(&mut stream, &mut reader)?,
        JSON_LB => json_auth(&mut stream, &mut reader)?,
        x => bail!("unknown identifying byte {}", x),
    }

    // Hand the TCP socket back to the Electrum server
    Ok(stream)
}

/// Wrap filter for HTTP basic authentication
#[cfg(feature = "http")]
pub fn http_basic_auth(
    access_token: Option<String>,
) -> warp::filters::BoxedFilter<(Result<(), Error>,)> {
    use bitcoin::base64;
    use std::sync::Arc;
    use warp::http::StatusCode;
    use warp::Filter;

    fn parse_header(header_val: String) -> Option<(String, String)> {
        if header_val.to_ascii_lowercase().starts_with("basic ") {
            let auth_base64 = &header_val[6..];
            let auth_decoded = String::from_utf8(base64::decode(&auth_base64).ok()?).ok()?;
            let mut parts = auth_decoded.splitn(2, ':');
            Some((parts.next()?.into(), parts.next()?.into()))
        } else {
            None
        }
    }

    if let Some(access_token) = access_token {
        let access_token = Arc::new(access_token);
        warp::any()
            .and(warp::any().map(move || access_token.clone()))
            .and(warp::header::optional("authorization"))
            .map(|access_token: Arc<String>, auth_header: Option<String>| {
                // We only care about the password, the username can be anything.
                let password = auth_header.and_then(parse_header).map(|creds| creds.1);
                ensure!(
                    password == Some(access_token.to_string()),
                    StatusCode::UNAUTHORIZED
                );
                Ok(())
            })
            .boxed()
    } else {
        // Return a pass-through filter if authentication is disabled
        warp::any().map(|| Ok(())).boxed()
    }
}
