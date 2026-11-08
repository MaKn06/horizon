#!/usr/bin/env bash
# Packages Aurora for macOS: Aurora.app (aurora, the Horizon launcher it drives beside it, the
# icon), signed ad hoc, inside a .dmg with an Applications link to drag it onto.
#
#   aurora/packaging/package.sh AURORA_BINARY HORIZON_LAUNCHER_BINARY VERSION OUTDIR ARCH
#
# Needs macOS (sips, iconutil, codesign, hdiutil). ARCH (arm64 or x64) only names the .dmg.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
aurora=$1
launcher=$2
version=$3
out=$(mkdir -p "$4" && cd "$4" && pwd)
arch=$5
app=$out/Aurora.app

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$aurora" "$app/Contents/MacOS/aurora"
cp "$launcher" "$app/Contents/MacOS/horizon-launcher"
chmod 755 "$app/Contents/MacOS/aurora" "$app/Contents/MacOS/horizon-launcher"

# The icon: every size iconutil wants, from the 1024 px PNG.
set_dir=$(mktemp -d)/Aurora.iconset
mkdir -p "$set_dir"
for size in 16 32 128 256 512; do
  sips -z $size $size "$here/icon.png" --out "$set_dir/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$here/icon.png" --out "$set_dir/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$set_dir" -o "$app/Contents/Resources/aurora.icns"

sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
plutil -lint "$app/Contents/Info.plist"

# Ad hoc: no Apple account needed. The launcher inside first, then the app around it.
codesign --force --sign - "$app/Contents/MacOS/horizon-launcher"
codesign --force --sign - "$app"
codesign --verify --strict --verbose=2 "$app"
"$app/Contents/MacOS/horizon-launcher" --version

stage=$(mktemp -d)/Aurora
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
dmg=$out/Aurora-$version-macos-$arch.dmg
rm -f "$dmg"
hdiutil create -volname Aurora -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
hdiutil verify "$dmg" >/dev/null
rm -rf "$app"
ls -l "$out"
