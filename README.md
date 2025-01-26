[![Crates.io](https://img.shields.io/crates/v/simagef.svg)](https://crates.io/crates/simagef)
[![AUR version](https://img.shields.io/aur/version/simagef-bin)](https://aur.archlinux.org/packages/simagef-bin)

`simagef` is a CLI tool for finding similar images. It takes a list of image paths and returns either groups or pairs of images that are similar.

It uses the [image-match](https://crates.io/crates/image-match) crate to generate signatures for images and compare them. An option to match images using pixel comparisons via the [image-compare](https://crates.io/crates/image-compare) crate is also available with the `pixel` feature flag.

`simagef` is faster than the [findimagedupes](https://github.com/jhnc/findimagedupes) Perl script, but might not produce identical results - not even across the signature/pixel modes, and is not designed to be a drop-in replacement.

## Performance

Comparing 10,198 small to medium sized images, simagef takes 40 seconds while findimagedupes takes 1 minute 44 seconds.

![](https://i.gyazo.com/4a3508da851cf2b2b731223571a3782b.png)

## Installation

Install via cargo on most platforms:

```
cargo install simagef
```

or with the pixel feature:

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

You can additionally read filenames from `stdin` if `-` appears in the filenames list. For example using it with the `fd` command for finding files:

```
fd . ~/my_images | simagef base.png base2.png -
```

You can use the (slower) pixel-based algorithm with the `-m` or `--pixels` flag.

If you want only the pairs of images without the groupings, use the `-p` or `--pairs` flag.

You can specify an external image viewer for comparing groups of images using `-e` or `--exec`. You can also provide command line arguments:

```
simagef -e "gwenview -s" ~/Pictures/*
```

This will launch the executable with the groups (or pairs) of filenames as arguments. It will launch the executable again for the next group once the previous executable exits.

## Caveats

- The `crossbeam` channels used in the code are unbounded, which I consider to be a bug.

- The groups are created using a recursive graph algorithm.
