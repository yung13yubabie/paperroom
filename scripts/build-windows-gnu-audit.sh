#!/usr/bin/env bash
# Cross-build a pinned application commit twice, retaining only sanitized evidence.
set -Eeuo pipefail

source_commit="${2:?Provide the full application source commit SHA}"
[[ "$source_commit" =~ ^[0-9a-f]{40}$ ]]
target=x86_64-pc-windows-gnu
rust_toolchain="${PAPERROOM_RUST_TOOLCHAIN:-1.100.0}"
repo="$(git rev-parse --show-toplevel)"
output="$(realpath -- "$1")"
test ! -e "$output"
test "$(uname -s -m)" = 'Linux x86_64'
test "$(git -C "$repo" rev-parse "$source_commit^{commit}")" = "$source_commit"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
mkdir "$work/debs" "$work/gnu" "$work/metadata"
export LC_ALL=C TZ=UTC CARGO_INCREMENTAL=0
export SOURCE_DATE_EPOCH="$(git -C "$repo" log -1 --format=%ct "$source_commit")"
export CARGO_BUILD_JOBS=2
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
export CARGO_PROFILE_RELEASE_LTO=false
seed_cargo="${CARGO_HOME:-$HOME/.cargo}"
export CARGO_HOME="$work/cargo-cache"
mkdir "$CARGO_HOME"
if [[ "${PAPERROOM_ALREADY_FETCHED:-0}" == 1 ]]; then
    cp -a --reflink=auto "$seed_cargo/registry" "$CARGO_HOME/"
fi
rustc "+$rust_toolchain" -vV > "$work/metadata/rustc.txt"
grep -qx 'release: 1.100.0' "$work/metadata/rustc.txt"
grep -qx 'commit-hash: 0ef1bc386059596dada8fb65b50425aab2ea637a' "$work/metadata/rustc.txt"
installed_sysroot="$(rustc "+$rust_toolchain" --print sysroot)"
sysroot="$work/rust-sysroot"
ln -s "$installed_sysroot" "$sysroot"
test -d "$sysroot/lib/rustlib/$target/lib"

# Debian trixie amd64 Packages index at 20261117T190000Z, release-hash-pinned files.
cat > "$work/metadata/debian-packages.txt" <<'PINS'
b53f4a091b16a0d0952b8e3bd6027931641386258f96b37db1f0c62c787221d3 pool/main/b/binutils-mingw-w64/binutils-mingw-w64-x86-64_2.44-3+12+b1_amd64.deb
8d2c64b886ab4a435a78ddc4cd3b510f149b65fbd6be8472e8f460627562718a pool/main/g/gcc-mingw-w64/gcc-mingw-w64-base_14.2.0-19+27+b1_amd64.deb
c898d96177574a8a7238564a1e799d8ddfed5a3d2ea005510305f9ad466a6f59 pool/main/g/gcc-mingw-w64/gcc-mingw-w64-x86-64-posix-runtime_14.2.0-19+27+b1_amd64.deb
e32d24cfd1482394f16d9a84c1d0a9f5aa08ee826a79acc383c70cc8c23c4fe5 pool/main/g/gcc-mingw-w64/gcc-mingw-w64-x86-64-posix_14.2.0-19+27+b1_amd64.deb
97dce5d0d8aff1cade4786ad4ef5078345477a598829afcac2e7f2fbc5399990 pool/main/m/mingw-w64/mingw-w64-common_12.0.0-5_all.deb
0bf89cf7454cccb49cd2a46a6f7b33896f6ae3a1a2fe33e02c40be6155bbb385 pool/main/m/mingw-w64/mingw-w64-x86-64-dev_12.0.0-5_all.deb
PINS
while read -r digest relative; do
    name="${relative##*/}"
    path="$work/debs/$name"
    if [[ -n "${PAPERROOM_DEB_CACHE:-}" ]]; then
        cp -- "$PAPERROOM_DEB_CACHE/$name" "$path"
    else
        curl --fail --silent --show-error --location --max-time 75 --output "$path" \
            "https://deb.debian.org/debian/$relative" ||
        curl --fail --silent --show-error --location --retry 2 --max-time 75 --output "$path" \
            "https://snapshot.debian.org/archive/debian/20261117T190000Z/$relative"
    fi
    printf '%s  %s\n' "$digest" "$path" | sha256sum --check --status
    dpkg-deb --extract "$path" "$work/gnu"
done < "$work/metadata/debian-packages.txt"

cross="$work/gnu/usr/bin"
ln -s x86_64-w64-mingw32-gcc-posix "$cross/x86_64-w64-mingw32-gcc"
export PATH="$cross:$PATH"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER="$cross/x86_64-w64-mingw32-gcc"
export CC_x86_64_pc_windows_gnu="$CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER"
export AR_x86_64_pc_windows_gnu="$cross/x86_64-w64-mingw32-ar"
"$CC_x86_64_pc_windows_gnu" -dumpfullversion -dumpversion > "$work/metadata/gcc.txt"
"$cross/x86_64-w64-mingw32-ld" --version > "$work/metadata/ld.txt"
git -C "$repo" archive --format=tar --output="$work/source.tar" "$source_commit"
for label in proof_a proof_b; do
    mkdir -p "$work/$label/src" "$work/$label/out"
    tar -xf "$work/source.tar" -C "$work/$label/src"
done
if [[ "${PAPERROOM_ALREADY_FETCHED:-0}" != 1 ]]; then
    cargo "+$rust_toolchain" fetch --locked --manifest-path "$work/proof_a/src/Cargo.toml"
fi

# GNU ld sorts the *archive path* for .idata$2 through .idata$7. Keep
# the independent output roots adjacent, on the same side of every
# linker input prefix. The first differing component of each input path
# is then independent of the build label and the host's cache location.
# Both compiler invocations get the same mapping list. Only one source and
# output root can match a given path, and the last matching prefix wins.
export RUSTFLAGS="--sysroot=$sysroot --remap-path-prefix=$work=/build-temp --remap-path-prefix=$CARGO_HOME=/cargo --remap-path-prefix=$sysroot=/toolchain --remap-path-prefix=$installed_sysroot=/toolchain"
export CFLAGS_x86_64_pc_windows_gnu="-ffile-prefix-map=$work=/build-temp -ffile-prefix-map=$CARGO_HOME=/cargo -g0"
for label in proof_a proof_b; do
    RUSTFLAGS+=" --remap-path-prefix=$work/$label/src=/paperroom --remap-path-prefix=$work/$label/out=/target"
    CFLAGS_x86_64_pc_windows_gnu+=" -ffile-prefix-map=$work/$label/src=/paperroom -ffile-prefix-map=$work/$label/out=/target"
done

build() (
    label="$1"
    cd "$work/$label/src"
    export CARGO_TARGET_DIR="$work/$label/out"
    cargo "+$rust_toolchain" rustc --offline --locked --release --target "$target" --bin paperroom \
        -- -C link-arg=-Wl,--no-insert-timestamp -C link-arg=-static-libgcc -C link-arg=-Wl,-Map,final.map
    mv -- final.map "$work/$label/final.map"
)
build proof_a > "$work/proof_a/build.log" 2>&1 &
first=$!
build proof_b > "$work/proof_b/build.log" 2>&1 &
second=$!
status=0
wait "$first" || status=1
wait "$second" || status=1
if [[ "$status" != 0 ]]; then
    tail -n 45 "$work/proof_a/build.log" "$work/proof_b/build.log"
    exit 1
fi

artifact="out/$target/release/paperroom.exe"
exe="$work/proof_a/$artifact"
rebuild="$work/proof_b/$artifact"
if [[ -n "${PAPERROOM_REPRO_EVIDENCE_DIR:-}" ]]; then
    evidence="$(realpath -- "$PAPERROOM_REPRO_EVIDENCE_DIR")"
    mkdir -- "$evidence"
    for pair in "a:$exe" "b:$rebuild"; do
        label="${pair%%:*}"
        binary="${pair#*:}"
        python3 -B "$repo/scripts/windows-candidate.py" audit "$binary" --prefix "$work" --prefix "$installed_sysroot"
        cp -- "$binary" "$evidence/rebuild-$label.exe"
    done
    (cd "$evidence" && sha256sum rebuild-a.exe rebuild-b.exe > SHA256SUMS)
fi
if ! cmp -s -- "$exe" "$rebuild"; then
    printf 'The two fresh GNU EXEs differ.\n'
    python3 -B "$repo/scripts/package-windows-gnu-audit.py" compare "$exe" "$rebuild"
    exit 1
fi
exe_sha="$(sha256sum "$exe" | cut -d ' ' -f 1)"
printf 'Designated GNU candidate: %s\n' "$exe_sha"
cargo "+$rust_toolchain" metadata --offline --locked --filter-platform "$target" --format-version 1 \
    --manifest-path "$work/proof_a/src/Cargo.toml" > "$work/metadata/cargo.json"
python3 -B "$repo/scripts/windows-build-provenance.py" --target "$target" \
    --metadata "$work/metadata/cargo.json" --exe-sha256 "$exe_sha" \
    --rustc-info "$work/metadata/rustc.txt" --rust-sysroot "$sysroot" \
    > "$work/metadata/cargo-provenance.json"
prov_sha="$(sha256sum "$work/metadata/cargo-provenance.json" | cut -d ' ' -f 1)"
python3 -B "$repo/scripts/windows-registry-notices.py" --target "$target" \
    --exe "$exe" --exe-sha256 "$exe_sha" --provenance "$work/metadata/cargo-provenance.json" \
    --provenance-sha256 "$prov_sha" --lock "$work/proof_a/src/Cargo.lock" \
    --cargo-home "$CARGO_HOME" --output "$work/registry"

python3 -B "$repo/scripts/package-windows-gnu-audit.py" build \
    --work "$work" --sysroot "$sysroot" --commit "$source_commit" --output "$output"
