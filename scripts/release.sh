#!/bin/bash
set -xeo pipefail

docker_name=shesek/bwt
gh_repo=shesek/bwt

if ! git diff-index --quiet HEAD; then
  echo git working directory is dirty
  exit 1
fi

version=`cat Cargo.toml | egrep '^version =' | cut -d'"' -f2`

if [[ "$1" == "patch" ]]; then
  # bump the patch version by one
  version=`node -p 'process.argv[1].replace(/\.(\d+)$/, (_, v) => "."+(+v+1))' $version`
  sed -i 's/^version =.*/version = "'$version'"/' Cargo.toml
elif [[ "$1" != "nobump" ]]; then
  echo invalid argument, use "patch" or "nobump"
  exit 1
fi

# Extract unreleased changelog & update version number
changelog="`sed -nr '/^## (Unreleased|'$version' )/{n;:a;n;/^## /q;p;ba}' CHANGELOG.md`"
grep '## Unreleased' CHANGELOG.md > /dev/null \
  && sed -i "s/^## Unreleased/## $version - `date +%Y-%m-%d`/" CHANGELOG.md

sed -ir "s|bwt-[0-9.]\+-x86_64|bwt-$version-x86_64|g; s|/download/v[0-9.]\+/|/download/v$version/|;" README.md

echo -e "Releasing bwt v$version\n================\n\n$changelog\n\n"

echo Running cargo check and fmt...
cargo fmt -- --check
./scripts/check.sh

if [ -z "$SKIP_BUILD" ]; then
  echo Building executables...
  rm -rf dist/*

  pack() {
    pname=$1; name=$2; dir=${3:-$2}
    pushd dist
    if [ $pname == 'linux' ]; then tar -czhf $name.tar.gz $dir
    else zip -rq $name.zip $dir; fi
    popd
  }
  build() {
    pname=$1; name=$2; toolchain=$3; platform=$4; features=$5; ext=$6
    echo "- Building $name using $toolchain for $platform with features: $features"
    cargo +$toolchain build --release --target $platform --no-default-features --features "$features"
    mkdir -p dist/$name
    mv target/$platform/release/bwt$ext dist/$name
    strip dist/$name/bwt$ext
    cp README.md LICENSE dist/$name
    pack $pname $name
  }
  for cfg in linux,stable,x86_64-unknown-linux-gnu, \
             win,nightly,x86_64-pc-windows-gnu,.exe; do
    IFS=',' read pname toolchain platform ext <<< $cfg
    build $pname bwt-$version-x86_64-$pname $toolchain $platform 'http electrum webhooks track-spends' $ext
    build $pname bwt-$version-electrum_only-x86_64-$pname $toolchain $platform electrum $ext
  done

  echo - Building electrum plugin
  for pname in linux win; do
    name=bwt-$version-electrum_plugin-x86_64-$pname
    mkdir dist/$name
    cp contrib/electrum-plugin/*.py dist/$name
    cp dist/bwt-$version-electrum_only-x86_64-$pname/bwt* README.md LICENSE dist/$name
    # needs to be inside a directory with a name that matches the plugin module name for electrum to load it,
    # create a temporary link to get tar/zip to pack it properly. (can also be done for tar.gz with --transform)
    ln -s $name dist/bwt
    pack $pname $name bwt
    rm dist/bwt
  done

  rm -r dist/*/

  echo Making SHA256SUMS...
  (cd dist && sha256sum *) | gpg --clearsign --digest-algo sha256 > SHA256SUMS.asc
fi


if [ -z "$SKIP_GIT" ]; then
  echo Tagging...
  git add Cargo.{toml,lock} CHANGELOG.md SHA256SUMS.asc README.md
  git commit -S -m v$version
  git tag --sign -m "$changelog" v$version
  git branch -f stable HEAD

  echo Pushing to github...
  git push gh master stable
  git push gh --tags
fi

if [[ -z "$SKIP_UPLOAD" && -n "$GH_TOKEN" ]]; then
  echo Uploading to github...
  gh_auth="Authorization: token $GH_TOKEN"
  gh_base=https://api.github.com/repos/$gh_repo
  release_text="## Changelog"$'\n'$'\n'$changelog$'\n'$'\n'`cat scripts/release-footer.md | sed "s/VERSION/$version/g"`
  release_opt=`jq -n --arg version v$version --arg text "$release_text" \
    '{ tag_name: $version, name: $version, body: $text, draft:true }'`
  gh_release=`curl -sf -H "$gh_auth" $gh_base/releases/tags/v$version \
           || curl -sf -H "$gh_auth" -d "$release_opt" $gh_base/releases`
  gh_upload=`echo "$gh_release" | jq -r .upload_url | sed -e 's/{?name,label}//'`

  for file in SHA256SUMS.asc dist/*; do
    echo ">> Uploading $file"

    curl -f --progress-bar -H "$gh_auth" -H "Content-Type: application/octet-stream" \
         --data-binary @"$file" "$gh_upload?name=$(basename $file)" | (grep -v browser_download_url || true)
  done

  # make release public once everything is ready
  curl -sf -H "$gh_auth" -X PATCH $gh_base/releases/`echo "$gh_release" | jq -r .id` \
    -d '{"draft":false}' > /dev/null
fi

if [ -z "$SKIP_DOCKER" ]; then
  echo Releasing docker images...

  docker_tag=$docker_name:$version
  docker build -t $docker_tag .
  docker build -t $docker_tag-electrum --build-arg FEATURES=electrum .
  docker tag $docker_tag $docker_name:latest
  docker tag $docker_tag-electrum $docker_name:electrum
  docker push $docker_name
fi

if [ -z "$SKIP_CRATE" ]; then
  echo Publishing to crates.io...
  cargo publish
fi
