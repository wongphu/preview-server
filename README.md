# preview-server

A small static file server for previewing web pages during development.

- **Live reload**: the page refreshes when files in the served directory change.
- **CSS hot-swap**: when only `.css` files change, stylesheets reload in place without a page refresh.
- **Directory listing** for folders without an `index.html`.
- **SPA fallback** (`--spa`): unknown extensionless paths such as `/users/42` serve `/index.html`.

## Install

```sh
cargo install --path .
```

## Usage

```sh
preview-server [DIR] [OPTIONS]

  -p, --port <PORT>   Port to listen on; the next free port is used if it is taken [default: 8080]
      --host <HOST>   Address to bind; use 127.0.0.1 to accept only local connections [default: 0.0.0.0]
  -o, --open          Open the browser after starting
      --spa           Serve /index.html for unknown extensionless paths
      --no-reload     Disable file watching and live reload
      --no-listing    Disable directory listings
  -q, --quiet         Don't log requests
```

## How it works

HTML responses get a `<script src="/__preview/reload.js">` tag injected before `</body>`.
That script subscribes to `/__preview/events`, a server-sent events stream that publishes
the URL paths of changed files (debounced by 100ms). Changes under `.git/` and
`node_modules/`, and editor swap files, are ignored.

All responses carry `Cache-Control: no-cache`. Requests that resolve outside the
served directory, including through symlinks, return 404.

## License

MIT. See [LICENSE](LICENSE).
