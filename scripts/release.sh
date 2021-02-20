#!/bin/bash
set -xeo pipefail
shopt -s expand_aliases

docker_name=shesek/bwt
gh_repo=bwt-dev/bwt

if ! git diff-index --quiet HEAD; then
  echo git working directory is dirty
  exit 1
fi

if [ -z "$BWT_BASE" ]; then
  echo >&2 BWT_BASE is required
  exit 1
fi

version=$(grep -E '^version =' Cargo.toml | cut -d'"' -f2)

if [[ "$1" == "patch" ]]; then
  # bump the patch version by one
  version=$(node -p 'process.argv[1].replace(/\.(\d+)$/, (_, v) => "."+(+v+1))' $version)
  sed -i 's/^version =.*/version = "'$version'"/' Cargo.toml
elif [[ "$1" != "nobump" ]]; then
  echo invalid argument, use "patch" or "nobump"
  exit 1
fi

# Extract unreleased changelog & update version number
changelog=$(sed -nr '/^## (Unreleased|'$version' )/{n;:a;n;/^## /q;p;ba}' CHANGELOG.md)
grep '## Unreleased' CHANGELOG.md > /dev/null \
  && sed -i "s/^## Unreleased/## $version - $(date +%Y-%m-%d)/" CHANGELOG.md

sed -i -r "s~bwt-[0-9a-z.-]+-x86_64-linux\.~bwt-$version-x86_64-linux.~g; s~/(download)/v[0-9a-z.-]+~/\1/v$version~;" README.md

echo -e "Releasing bwt v$version\n================\n\n$changelog\n\n"

echo Running cargo check and fmt...
cargo fmt -- --check
./scripts/check.sh

alias docker_run="docker run -it --rm -u $(id -u) -v $(pwd):/usr/src/bwt \
    -v ${CARGO_HOME:-$HOME/.cargo}:/usr/local/cargo \
    -v ${SCCACHE_DIR:-$HOME/.cache/sccache}:/usr/local/sccache"

if [ -z "$SKIP_BUILD" ]; then
  echo Building releases...

  mkdir -p dist
  rm -rf dist/*

  if [ -z "$BUILD_HOST" ]; then
    docker build -t bwt-builder - < scripts/builder.Dockerfile
    docker_run bwt-builder
    if [ -z "$SKIP_OSX" ]; then
      docker build -t bwt-builder-osx - < scripts/builder-osx.Dockerfile
      docker_run bwt-builder-osx
    fi
  else
    # macOS builds are disabled by default when building on the host.
    # to enable, set TARGETS=x86_64-osx,...
    ./scripts/build.sh
  fi

  echo Making SHA256SUMS...
  (cd dist && sha256sum *.{tar.gz,zip}) | sort | gpg --clearsign --digest-algo sha256 > SHA256SUMS.asc
fi

if [ -z "$SKIP_GIT" ]; then
  echo Tagging...
  git add Cargo.{toml,lock} CHANGELOG.md SHA256SUMS.asc README.md
  git commit -S -m v$version
  git tag --sign -m "$changelog" v$version
  git branch -f latest HEAD

  echo Pushing to github...
  git push gh master latest
  git push gh --tags
fi

if [ -z "$SKIP_CRATE" ]; then
  echo Publishing to crates.io...
  cargo publish
fi

if [[ -z "$SKIP_UPLOAD" && -n "$GH_TOKEN" ]]; then
  echo Uploading to github...
  gh_auth="Authorization: token $GH_TOKEN"
  gh_base=https://api.github.com/repos/$gh_repo

  travis_job=$(curl -s "https://api.travis-ci.org/v3/repo/${gh_repo/\//%2F}/branch/v$version" | jq -r '.last_build.id // ""')

  release_text="### Changelog"$'\n'$'\n'$changelog$'\n'$'\n'$(sed "s/VERSION/$version/g; s/TRAVIS_JOB/$travis_job/g;" scripts/release-footer.md)
  release_opt=$(jq -n --arg version v$version --arg text "$release_text" \
    '{ tag_name: $version, name: $version, body: $text, draft:true }')
  gh_release=$(curl -sf -H "$gh_auth" $gh_base/releases/tags/v$version \
           || curl -sf -H "$gh_auth" -d "$release_opt" $gh_base/releases)
  gh_upload=$(echo "$gh_release" | jq -r .upload_url | sed -e 's/{?name,label}//')

  for file in SHA256SUMS.asc dist/*.{tar.gz,zip}; do
    echo ">> Uploading $file"

    curl -f --progress-bar -H "$gh_auth" -H "Content-Type: application/octet-stream" \
         --data-binary @"$file" "$gh_upload?name=$(basename $file)" | (grep -v browser_download_url || true)
  done

  # make release public once everything is ready
  curl -sf -H "$gh_auth" -X PATCH "$gh_base/releases/$(echo "$gh_release" | jq -r .id)" \
    -d '{"draft":false}' > /dev/null
fi

if [ -z "$SKIP_DOCKER" ]; then
  echo Releasing docker images...
  ./scripts/docker-release.sh
fi

if [ -z "$SKIP_SUBPROJECTS" ]; then
  export BWT_COMMIT=$(git rev-parse HEAD)

  echo '## Releasing libbwt'
  (cd $BWT_BASE/libbwt && ./scripts/release.sh)

  echo '## Releasing libbwt-jni'
  (cd $BWT_BASE/libbwt-jni && ./scripts/release.sh)

  echo '## Releasing bwt-electrum-plugin'
  (cd $BWT_BASE/bwt-electrum-plugin && ./scripts/release.sh)

  echo '## Releasing libbwt-nodejs'
  export LIBBWT_COMMIT=$(cd $BWT_BASE/libbwt && git rev-parse HEAD)
  (cd $BWT_BASE/libbwt-nodejs && ./scripts/release.sh)
fi
