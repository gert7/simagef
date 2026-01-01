[![Crates.io](https://img.shields.io/crates/v/simagef.svg)](https://crates.io/crates/simagef)
[![AUR version](https://img.shields.io/aur/version/simagef-bin)](https://aur.archlinux.org/packages/simagef-bin)

`simagef` is a CLI tool for finding similar images. It takes a list of image
paths and returns either groups or pairs of images that are similar.

It uses the [image-match](https://crates.io/crates/image-match) crate to
generate signatures for images and compare them. An option to match images using
pixel comparisons via the [image-compare](https://crates.io/crates/image-compare)
crate is also available with the `pixel` feature flag.

`simagef` is faster than the
[findimagedupes](https://github.com/jhnc/findimagedupes) Perl script, but might
not produce identical results - not even across the signature/pixel modes, and
is not designed to be a drop-in replacement.

## Performance

Comparing 16,664 small to medium sized images, simagef takes 1 minute 31 seconds
on the first run.

![Screenshot_20250606_185139](https://github.com/user-attachments/assets/302f67df-9479-458f-a6c5-481e40f39b6b)

## Installation

Install via cargo on most platforms:

```
cargo install simagef
```

For AVIF support, requiring
[libdav1d](https://github.com/videolan/dav1d) to be installed on your system:

```
cargo install simagef --features avif
```

The less efficient "pixel" mode:

```
cargo install simagef --features pixel
```

Install on Arch Linux:

```
yay simagef-bin
```

## Usage

To compare a set of images:

```
simagef a.png b.png c.png
```

You can set the threshold of similarity with the `-t` or `--threshold` options:

```
simagef -t 50 a.png b.png c.png
```

You can additionally read filenames from `stdin` if `-` appears in the filenames
list. For example using it with the [fd](https://github.com/sharkdp/fd)
command for finding files:

```
fd . ~/my_images | simagef base.png base2.png -
```

You can use the (slower) pixel-based algorithm with the `-m` or `--pixels` flag.

If you want only the pairs of images without the groupings, use the `-p` or
`--pairs` flag.

You can specify an external image viewer for comparing groups of images using
`-e` or `--exec`. You can also provide command line arguments:

```
simagef -e "gwenview -s" ~/Pictures/*
```

This will launch the executable with the groups (or pairs) of filenames as
arguments. It will launch the executable again for the next group once the
previous executable exits.

### Formatting

Use the `--format` option to specify how output to stdout should be formatted:

- `--format regular` separates file paths with spaces and groups with newlines.
It makes no distinction between path separation and spaces in paths.

- `--format quote` wraps filenames in quote marks, separates file paths with
spaces, and escapes spaces in paths with a literal backslash. It separates
groups with newlines.

- `--format null` provides file paths in full, separates file paths with the
NUL character and separates groups with two subsequent NUL characters.

### Database

From version 1.3.0, the database is enabled by default and greatly speeds up
subsequent runs in the default mode. Signatures will be stored in your cache
directory in a SQLite database file named `simagef`. You can disable this with
the `--no-database` option.

## Caveats

- The groups are created using a recursive graph algorithm.
