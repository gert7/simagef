[![Crates.io](https://img.shields.io/crates/v/simagef.svg)](https://crates.io/crates/simagef)

`simagef` is a CLI tool for finding similar images. It takes a list of image paths and returns either groups or pairs of images that are similar.

It uses the [image-match](https://crates.io/crates/image-match) crate to generate signatures for images and compare them. An option to match images using pixel comparisons via the [image-compare](https://crates.io/crates/image-compare) crate is also available.

`simagef` is around **8.5x faster** than the [findimagedupes](https://github.com/jhnc/findimagedupes) Perl script, but might not produce identical results - not even across the signature/pixel modes, and is not designed to be a drop-in replacement.

## Installation

Install on Arch Linux:

```
yay simagef-bin
```

Install via cargo:

```
cargo install simagef
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

You can additionally read filenames from `stdin` if `-` appears in the filenames list. For example using it with the `fd` command for finding files:

```
fd . ~/my_images | simages base.png base2.png -
```

You can use the (slower) pixel-based algorithm with the `-m` or `--pixels` flag.

If you want only the pairs of images without the groupings, use the `-p` or `--pairs` flag.

You can specify an external image viewer for comparing groups of images using `-e` or `--exec`. You can also provide command line arguments:

```
simagef -e "gwenview -s" ~/Pictures/*
```

This will launch the executable with the groups (or pairs) of filenames as arguments. It will launch the executable again for the next group once the previous executable exits.

## Caveats

- Comparing a large number of images will fill up your memory.

- The `crossbeam` channels used in the code are unbounded, which I consider to be a bug.

- The groups are created using a recursive graph algorithm.

