
------------

Also see the vVERSION downloads for [bwt-electrum-plugin](https://github.com/bwt-dev/bwt-electrum-plugin/releases/tag/vVERSION), [libbwt](https://github.com/bwt-dev/libbwt/releases/tag/vVERSION), [libbwt-nodejs](https://github.com/bwt-dev/libbwt-nodejs/releases/tag/vVERSION) and [libbwt-jni](https://github.com/bwt-dev/libbwt-jni/releases/tag/vVERSION).

### Installation

Installation instructions are [available on the README](https://github.com/shesek/bwt#installation).

### Verifying signatures

The releases are signed by Nadav Ivgi (@shesek).
The public key can be verified on
the [PGP WoT](http://keys.gnupg.net/pks/lookup?op=vindex&fingerprint=on&search=0x81F6104CD0F150FC),
[github](https://api.github.com/users/shesek/gpg_keys),
[twitter](https://twitter.com/shesek),
[keybase](https://keybase.io/nadav),
[hacker news](https://news.ycombinator.com/user?id=nadaviv)
and [this video presentation](https://youtu.be/SXJaN2T3M10?t=4) (bottom of slide).

```bash
# Download (change x86_64-linux to your platform)
$ wget https://github.com/shesek/bwt/releases/download/vVERSION/bwt-VERSION-x86_64-linux.tar.gz

# Fetch public key
$ gpg --keyserver keyserver.ubuntu.com --recv-keys FCF19B67866562F08A43AAD681F6104CD0F150FC

# Verify signature
$ wget -qO - https://github.com/shesek/bwt/releases/download/vVERSION/SHA256SUMS.asc \
  | gpg --decrypt - | grep ' bwt-VERSION-x86_64-linux.tar.gz$' | sha256sum -c -

$ tar zxvf bwt-VERSION-x86_64-linux.tar.gz
$ ./bwt-0.1.5-x86_64-linux/bwt --xpub <xpub> ...
```

The signature verification should show `Good signature from "Nadav Ivgi <nadav@shesek.info>" ... Primary key fingerprint: FCF1 9B67 ...` and `bwt-VERSION-x86_64-linux.tar.gz: OK`.

### Reproducible builds

The builds are fully reproducible.

You can verify the checksums against the vVERSION builds on Travis CI: https://travis-ci.org/github/shesek/bwt/builds/TRAVIS_JOB

See [more details here](https://github.com/shesek/bwt#reproducible-builds).
