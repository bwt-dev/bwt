------------

### Installation

Installation instructions are [available on the README](https://github.com/shesek/bwt#installation).

#### Verifying signatures

The releases are signed by Nadav Ivgi (@shesek). The public key can be verified on [keybase](https://keybase.io/nadav), [github](https://api.github.com/users/shesek/gpg_keys), [twitter](https://twitter.com/shesek) and [HN](https://news.ycombinator.com/user?id=nadaviv). The signature can be verified as follows (replace `x86_64-linux` with your download):

```bash
# Download package
$ wget https://github.com/shesek/bwt/releases/download/vVERSION/bwt-VERSION-x86_64-linux.tar.gz

# Verify signatures
$ gpg --keyserver keyserver.ubuntu.com --recv-keys FCF19B67866562F08A43AAD681F6104CD0F150FC
$ wget -qO - https://github.com/shesek/bwt/releases/download/vVERSION/SHA256SUMS.asc \
  | gpg --decrypt - | grep ' bwt-VERSION-x86_64-linux.tar.gz$' | sha256sum -c -
```

You should see `Good signature from "Nadav Ivgi <nadav@shesek.info>" ... Primary key fingerprint: FCF1 9B67 ...` and `bwt-VERSION-x86_64-linux.tar.gz: OK`.

#### Reproducible builds

The builds are fully reproducible.

You can verify the checksums against [the builds made on Travis CI](https://travis-ci.org/github/shesek/bwt) -- **doing so is highly recommended!**

See [more details here](https://github.com/shesek/bwt#reproducible-builds).

#### Electrum plugin

The [Electrum plugin](https://github.com/shesek/bwt#electrum-plugin) is available for download for Linux, Mac, Windows and ARM as the `electrum_plugin` package.

> ⚠️ **NOTE:** The plugin supports watch-only wallets only and **cannot be used with hot wallets**. This is done as a security measure, which is expected to eventually be lifted. You can use the plugin with hardware wallets or with an offline Electrum setup. For hot wallets, you will need to setup a standalone server instead of using the plugin, ideally far away from your keys.
