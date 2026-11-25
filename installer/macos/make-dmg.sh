#!/bin/sh
set -eu

binary="$1"
version="$2"
out="$3"
app_name="p5099"
display_name="P5099"
icon="$(cd "$(dirname "$0")/../.." && pwd)/assets/icon-macos.png"
work="$(mktemp -d)"
app="$work/dmg/$app_name.app"

mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$out"
cp "$binary" "$app/Contents/MacOS/$app_name"
chmod +x "$app/Contents/MacOS/$app_name"
iconset="$work/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$icon" --out "$iconset/icon_${size}x${size}.png" >/dev/null
done
for size in 16 32 128 256 512; do
    double=$((size * 2))
    sips -z "$double" "$double" "$icon" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$display_name</string>
  <key>CFBundleDisplayName</key><string>$display_name</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIdentifier</key><string>kr.p5099.launcher</string>
  <key>CFBundleExecutable</key><string>$app_name</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$app"
ditto -c -k --keepParent "$app" "$out/$app_name.app.zip"
ln -s /Applications "$work/dmg/Applications"
hdiutil create -volname "$display_name" -srcfolder "$work/dmg" -ov -format UDZO \
    "$out/P5099.dmg"
rm -rf "$work"
echo "built $out/P5099.dmg and $out/$app_name.app.zip"
