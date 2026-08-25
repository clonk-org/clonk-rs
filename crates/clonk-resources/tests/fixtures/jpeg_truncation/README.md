# Truncated-JPEG oracle fixtures

What libjpeg produces for the port's shared JPEG decoder, for
clonk-org/clonk-rs#987.

`oracle_main.cpp` mirrors `StdJpeg::Impl` (oracle `src/StdJpegLibjpeg.cpp:38-141`)
and the row loop of `C4Surface::ReadJPEG` (oracle `src/C4Surface.cpp:1029-1072`).
The load-bearing part is `fill_input_buffer`: when the input is exhausted it
hands libjpeg a synthetic `FF D9` end-of-image rather than reporting an error,
so a truncated entropy stream decodes to a **complete, full-size image**
instead of failing.

## Regenerating

```sh
clang++ -std=c++17 -O2 -I/opt/homebrew/include -L/opt/homebrew/lib -ljpeg \
  -o jpeg_oracle oracle_main.cpp
./jpeg_oracle gradient16.jpg        # full file  -> gradient16_libjpeg_full.rgb
./jpeg_oracle gradient16.jpg 640    # truncated  -> gradient16_libjpeg_keep640.rgb
```

Each run prints `{"width","height","rows_written","error","rgb":[…]}`; the
`.rgb` files are that `rgb` array written as raw bytes. Both runs report
`rows_written: 16` and an empty `error` — libjpeg never fails on this input.

`gradient16.jpg` is a 16x16 RGB gradient encoded at quality 90; its scan data
starts at byte 623, so keeping 640 bytes truncates 39 bytes into the entropy
stream. The truncation point matters: cutting into the header instead (below
byte 623) makes libjpeg throw `Bogus Huffman table definition`, which
`C4Surface::ReadJPEG` catches before the surface is ever created.

## What the fixtures pin, and what they cannot

`jpeg-decoder` is not `libjpeg`. Decoding the **untruncated** file already
differs from `gradient16_libjpeg_full.rgb` in 85 of 768 bytes (max delta 2) —
an inverse-DCT and colour-conversion difference that predates this issue and
applies to every JPEG the port reads.

Against that baseline the truncation recovery itself is effectively exact:
feeding the same synthetic `FF D9`, the port's decode of the truncated stream
differs from `gradient16_libjpeg_keep640.rgb` in only **3 of 768 bytes, by 1**.
The test therefore pins the relationship rather than byte-equality: recovering
from a truncated stream must not be further from libjpeg than decoding the
whole file already is.
