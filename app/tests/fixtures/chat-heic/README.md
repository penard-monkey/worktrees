# Chat HEIC fixtures

`orientation-6.heic` is a 120 × 80 four-quadrant image with EXIF orientation
6. WebKit must decode it as 80 × 120; after the clockwise turn its upper left
is blue and upper right is red. `large-3000x1600.heic` tests the 2048 px long
edge. Both files are small enough to commit.

To recreate them on macOS, with Pillow available for the source images:

```sh
python3 - <<'PY'
from PIL import Image, ImageDraw
im = Image.new('RGB', (120, 80), 'white')
large = Image.new('RGB', (3000, 1600), 'white')
for image in (im, large):
    d = ImageDraw.Draw(image)
    w, h = image.size
    d.rectangle((0, 0, w//2-1, h//2-1), fill='red')
    d.rectangle((w//2, 0, w-1, h//2-1), fill='green')
    d.rectangle((0, h//2, w//2-1, h-1), fill='blue')
    d.rectangle((w//2, h//2, w-1, h-1), fill='yellow')
exif = Image.Exif(); exif[274] = 6
im.save('orientation-source.jpg', quality=95, exif=exif)
large.save('large-source.png')
PY
sips -s format heic orientation-source.jpg --out orientation-6.heic
sips -s format heic large-source.png --out large-3000x1600.heic
```

The source files are disposable. `sips` preserves the JPEG's orientation in
the HEIC; `sips -g pixelWidth -g pixelHeight` reports its unrotated 120 × 80
dimensions, while WebKit's oriented bitmap measures 80 × 120.
