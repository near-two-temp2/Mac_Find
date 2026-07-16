# Embedded CJK font

`NotoSansCJKsc-Regular-subset.otf` is a subset of **Noto Sans CJK** (regular
weight), embedded by `internal/theme/theme.go` so the Fyne UI can render Chinese
text and CJK file paths (Fyne's bundled font is Latin-only → tofu boxes □).

## What's included

- Latin (U+0000–00FF) + general/CJK punctuation + Hiragana/Katakana + full/half
  width forms.
- CJK Unified Ideographs main block (U+4E00–9FFF) — covers common Simplified and
  Traditional Chinese, enough for UI strings and typical filenames.

Rare CJK extension blocks are intentionally dropped to keep the file small.

## Regenerating

Source is the variable font shipped with `github.com/go-text/typesetting-utils`
(a transitive Fyne dependency), so no network download is needed. With
`fonttools` installed:

```sh
SRC="$(go env GOMODCACHE)/github.com/go-text/typesetting-utils@*/opentype/common/NotoSansCJKjp-VF.otf"
# 1) glyph-subset (keeps the font variable)
python -m fontTools.subset "$SRC" \
  --unicodes="0000-00FF,2000-206F,3000-303F,3040-309F,30A0-30FF,FF00-FFEF,4E00-9FFF" \
  --output-file=/tmp/subset-var.otf --no-hinting --drop-tables+=DSIG
# 2) pin the weight axis to Regular (400) → a static face Fyne can load
python -m fontTools.varLib.instancer /tmp/subset-var.otf wght=400 \
  -o NotoSansCJKsc-Regular-subset.otf
```

Licensed under the SIL Open Font License 1.1 (Noto Sans CJK).
