# Authentication

## Enabling authentication

To enable authentication, set `--auth-cookie <file>` to generate a random access token and persist it to `<file>`.
You can print the generated token to `STDOUT` with `--print-token`, or grab it from the file.

Alternatively, you may use `--auth-token <token>` to specify your own access token.

Jump to: [Electrum](#electrum-authentication) &middot; [HTTP](#http-api-authentication)

## Electrum authentication

Electrum does not natively support authentication. The mechanism described below is a workaround that (ab)uses the fact
that some Electrum-backed wallets (including Electrum desktop/mobile, Sparrow, BitBoxApp and others) have built-in
SOCKS5 proxy support.

This workaround has two main limitations:

1. It can't work if you need to set a real proxy server.

2. The access token is sent ⚠️ UNENCRYPTED.

   While fine for LAN or local use, this is *not* recommended for internet-exposed servers.
   An attacker that can see your network traffic will be able to extract your access token and
   check whether certain addresses are associated with your wallet, by querying for their history and 
   checking if the server knows about them or not.

   If you are setting up a server reachable over the internet, you should set it up behind a secure transport
   like an SSH tunnel, a Tor hidden service or a VPN.
   If you're already taking the risk of having an entirely unauthenticated personal Electrum server exposed to the internet
   and understand the risks involved, using this may provide some additional protection against some types of attackers.

Enabling this feature requires setting `--electrum-socks-auth` in addition to the `--auth-*` options.

### Wallet setup

To authenticate, you will need to enable SOCKS5 proxy in your wallet, configure the SOCKS5 proxy address to your *server address*
and the SOCKS5 password to your access token.

The server address/port itself can be set to anything, but will need to end with a `:t` if you don't have SSL enabled.

Example configuration:

- SOCKS5 proxy address: `192.168.1.106:50001` (the *server* address)
- SOCKS5 username: `bwt` (can be anything)
- SOCKS5 password: `mySecretAccessToken`
- Server address: `bwt:1:t` (hostname/port can be anything)

> For wallets that support setting a proxy but not a password for it, the token may alternatively be provided as the server hostname.

This is how it looks like with Electrum desktop:

![Setting up Electrum desktop with authentication](https://raw.githubusercontent.com/bwt-dev/bwt/master/doc/img/electrum-auth-desktop.png)


Or with Electrum mobile:

![Setting up Electrum mobile with authentication](https://raw.githubusercontent.com/bwt-dev/bwt/master/doc/img/electrum-auth-mobile.png)


### How does this work?

To enable SOCKS5-based authentication, the bwt Electrum server masquerades as a SOCKS5 proxy server, parses the SOCKS5 handshake,
verifies the token, then passes control over the TCP socket to the 'real' server and continues processing the connection as usual.

The implementation can be seen in [`auth.rs`](https://github.com/bwt-dev/bwt/blob/master/src/util/auth.rs) under `electrum_socks5_auth`.

### Authenticating programmatically

Some Electrum client libraries support setting a proxy server, which can be used to authenticate with the server.
Example with [`rust-electrum-client`](https://github.com/bitcoindevkit/rust-electrum-client):

```rust
use electrum_client::{Client, ConfigBuilder, Socks5Config};

let socks5 = Socks5Config::with_credentials("192.168.1.106:50001", "bwt".into(), "mySecretAccessToken".into());
let config = ConfigBuilder::new().socks5(Some(socks5))?.build();
let client = Client::from_config("0.0.0.0:1", config)?;
```

Authenticating with `ncat`:

```bash
$ echo '{"method":"server.version"}' | ncat --proxy-type socks5 --proxy 192.168.1.106:50001 --proxy-auth bwt:mySecretAccessToken 0.0.0.0 1
```

Authenticating with `nc` using hostname-based authentication (`nc` does not support SOCKS5 authentication):

```bash
$ echo '{"method":"server.version"}' | nc -x 192.168.1.106:50001 mySecretAccessToken 1
```

If you'd like to authenticate without using a SOCKS5 implementation, you can also send the following byte sequence directly, then read and discard the first 14 bytes of the response.

```bash
0x05 0x01 0x02 0x01 0x00 <LENGTH> <TOKEN> 0x05 0x01 0x00 0x01 0x00 0x00 0x00 0x00 0x00 0x00
```

(Where `<LENGTH> <TOKEN>` is a single byte with the token length followed by the token itself.)

## HTTP API authentication

Using HTTP basic authentication headers, with an empty (or any) username and the password set to the access token.

Example with `curl`:

```bash
$ curl -u :mySecretAccessToken http://192.168.1.106:3060/wallets
```

If exposed to the public internet, the HTTP API server should be put behind a reverse proxy with SSL.
