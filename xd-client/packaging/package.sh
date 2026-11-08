#!/usr/bin/env bash
# Packages xD Client for macOS: "xD Client.app" (xd-client, Horizon's launcher it drives beside
# it, the icon), signed ad hoc, inside a .dmg with an Applications link to drag it onto.
#
#   xd-client/packaging/package.sh CLIENT_BINARY HORIZON_LAUNCHER_BINARY VERSION OUTDIR ARCH
#
# Needs macOS (sips, iconutil, codesign, hdiutil). ARCH (arm64 or x64) only names the .dmg.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
client=$1
launcher=$2
version=$3
out=$(mkdir -p "$4" && cd "$4" && pwd)
arch=$5
app="$out/xD Client.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$client" "$app/Contents/MacOS/xd-client"
cp "$launcher" "$app/Contents/MacOS/horizon-launcher"
chmod 755 "$app/Contents/MacOS/xd-client" "$app/Contents/MacOS/horizon-launcher"

# The icon: every size iconutil wants, from the 1024 px PNG.
set_dir=$(mktemp -d)/xd-client.iconset
mkdir -p "$set_dir"
for size in 16 32 128 256 512; do
  sips -z $size $size "$here/icon.png" --out "$set_dir/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) "$here/icon.png" --out "$set_dir/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$set_dir" -o "$app/Contents/Resources/xd-client.icns"

sed "s/@VERSION@/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
printf 'APPL????' > "$app/Contents/PkgInfo"
plutil -lint "$app/Contents/Info.plist"

# Ad hoc: no Apple account needed. Horizon's official launcher keeps its own signature; one
# without any is signed ad hoc first. Then the app around it.
codesign --verify "$app/Contents/MacOS/horizon-launcher" 2>/dev/null \
  || codesign --force --sign - "$app/Contents/MacOS/horizon-launcher"
codesign --force --sign - "$app"
codesign --verify --strict --verbose=2 "$app"
"$app/Contents/MacOS/horizon-launcher" --version

stage=$(mktemp -d)/xd-client
mkdir -p "$stage"
cp -R "$app" "$stage/"
ln -s /Applications "$stage/Applications"
dmg="$out/xD-Client-$version-macos-$arch.dmg"
rm -f "$dmg"
hdiutil create -volname "xD Client" -srcfolder "$stage" -ov -format UDZO "$dmg" >/dev/null
hdiutil verify "$dmg" >/dev/null
rm -rf "$app"
ls -l "$out"
