#!/usr/bin/env bash
# Renders the app icon (macos/Icon/AppIcon.svg) into macos/Isomar/Assets.xcassets/AppIcon.appiconset, and the
# iPhone and iPad one (macos/Icon/AppIconTouch.svg, full bleed and opaque) into macos/IsomarTouch/Assets.xcassets.
# The PNGs are committed, so the Mac build needs no SVG renderer; run this after changing the SVG.
# Needs rsvg-convert (librsvg) or ImageMagick built with librsvg.
set -euo pipefail
cd "$(dirname "$0")/.."
svg=Icon/AppIcon.svg
out=Isomar/Assets.xcassets/AppIcon.appiconset
mkdir -p "$out"

render() {   # render <px> <file>
    if command -v rsvg-convert >/dev/null; then
        rsvg-convert -w "$1" -h "$1" "$svg" -o "$2"
    else
        magick -background none "$svg" -resize "$1x$1" -depth 8 "PNG32:$2"
    fi
}

images=""
for size in 16 32 128 256 512; do
    for scale in 1 2; do
        px=$(( size * scale ))
        suffix=""; [ "$scale" = 1 ] || suffix=@2x
        name="icon_${size}x${size}${suffix}.png"
        render "$px" "$out/$name"
        images+="    { \"idiom\": \"mac\", \"size\": \"${size}x${size}\", \"scale\": \"${scale}x\", \"filename\": \"$name\" },"$'\n'
    done
done
cat > "$out/Contents.json" <<JSON
{
  "images": [
${images%,$'\n'}
  ],
  "info": { "author": "xcode", "version": 1 }
}
JSON
cat > Isomar/Assets.xcassets/Contents.json <<JSON
{
  "info": { "author": "xcode", "version": 1 }
}
JSON
ls "$out"

# iOS takes one 1024 px image with no alpha and scales it; the system rounds the corners.
touch=IsomarTouch/Assets.xcassets/AppIcon.appiconset
mkdir -p "$touch"
if command -v rsvg-convert >/dev/null; then
    rsvg-convert -w 1024 -h 1024 -b '#e6f1f9' Icon/AppIconTouch.svg -o "$touch/icon_1024.png"
else
    magick -background '#e6f1f9' Icon/AppIconTouch.svg -resize 1024x1024 -alpha remove -alpha off -depth 8 "PNG24:$touch/icon_1024.png"
fi
cat > "$touch/Contents.json" <<JSON
{
  "images": [
    { "idiom": "universal", "platform": "ios", "size": "1024x1024", "filename": "icon_1024.png" }
  ],
  "info": { "author": "xcode", "version": 1 }
}
JSON
cat > IsomarTouch/Assets.xcassets/Contents.json <<JSON
{
  "info": { "author": "xcode", "version": 1 }
}
JSON
ls "$touch"
