# This is a tool made for myself. Patch it for your use case if you want to use it

![Works on my machine](https://blog.codinghorror.com/content/images/uploads/2007/03/6a0120a85dcdae970b0128776ff992970c-pi.png)

# Poster

Background daemon + composer architecture.
Do not need to login everytime you want to post.

## Dependencies

- wayland
- notify-send
- neovim

## Daemon

```sh
cargo r --release --bin daemon
```

Reads `$handle` and `$password` from environment on the first time or whenever
session json is failing.

## Composer

```sh
cargo r --release --bin composer
```

Runs neovim to write post on a temporary file. Images can be added at the end by
specifying paths

```
てすてす
---
/path/to/image.jpg
[clipboard]
```

Image can be read from clipboard. Requires wayland.
