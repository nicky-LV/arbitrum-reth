#!/usr/bin/env bash
# Serve (or build) The arbitrum-reth Book.
#
#   ./open_book.sh                 serve on 127.0.0.1:3001 (or the next free port)
#   ./open_book.sh --port 4000     serve on exactly this port, or fail if it is taken
#   ./open_book.sh --host 0.0.0.0  bind all interfaces (reachable over the network)
#   ./open_book.sh build           build static HTML into book/book/ and exit
#
# Any other arguments are passed straight through to mdbook.

set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
book_dir="$repo_root/book"

if ! command -v mdbook >/dev/null 2>&1; then
    echo "error: mdbook is not installed." >&2
    echo "install it with:  cargo install mdbook --locked" >&2
    exit 1
fi

if [[ ! -f "$book_dir/book.toml" ]]; then
    echo "error: no book.toml under $book_dir" >&2
    exit 1
fi

# `build` (or `--build`) does a one-shot render instead of serving.
if [[ "${1:-}" == "build" || "${1:-}" == "--build" ]]; then
    shift
    mdbook build "$book_dir" "$@"
    echo "book written to $book_dir/book"
    exit 0
fi

host="127.0.0.1"
# 3000 is mdbook's own default but collides with too much else. Start at 3001 and walk
# upward until something is free, so a busy box does not need a remembered port number.
port="3001"
port_was_explicit=0
args=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        -p|--port)  port="$2"; port_was_explicit=1; shift 2 ;;
        -H|--host)  host="$2"; shift 2 ;;
        *)          args+=("$1"); shift ;;
    esac
done

# mdbook panics rather than reporting a bound port, so resolve the port ourselves first.
probe_host="$host"
[[ "$probe_host" == "0.0.0.0" || "$probe_host" == "::" ]] && probe_host="127.0.0.1"

port_is_taken() {
    # A successful connect means something is listening. Also probe the wildcard: a
    # listener on *:PORT blocks the bind even when 127.0.0.1:PORT looks quiet.
    (exec 3<>"/dev/tcp/${probe_host}/$1") 2>/dev/null && { exec 3<&- 3>&-; return 0; }
    (exec 3<>"/dev/tcp/0.0.0.0/$1") 2>/dev/null && { exec 3<&- 3>&-; return 0; }
    return 1
}

if [[ "$port_was_explicit" == 1 ]]; then
    # An explicit --port is a request, not a hint: fail loudly instead of moving.
    if port_is_taken "$port"; then
        echo "error: something is already listening on ${probe_host}:${port}." >&2
        exit 1
    fi
else
    start_port="$port"
    for _ in $(seq 1 25); do
        port_is_taken "$port" || break
        port=$((port + 1))
    done
    if port_is_taken "$port"; then
        echo "error: no free port in ${start_port}-${port}. Pass --port explicitly." >&2
        exit 1
    fi
    [[ "$port" != "$start_port" ]] &&
        echo "port ${start_port} is busy; using ${port} instead."
fi

# `mdbook serve --open` shells out to a browser. On a headless box (an SSH session
# with no display) that just prints an error, so only ask for it when something can
# plausibly handle it.
open_flag=()
if [[ -n "${BROWSER:-}" || -n "${DISPLAY:-}" || -n "${WAYLAND_DISPLAY:-}" || "$(uname -s)" == "Darwin" ]]; then
    open_flag=(--open)
else
    echo "no display detected; not launching a browser."
    if [[ "$host" == "127.0.0.1" || "$host" == "localhost" ]]; then
        echo "forward the port to read it locally:"
        echo "    ssh -L ${port}:127.0.0.1:${port} ${USER:-you}@$(hostname -f 2>/dev/null || hostname)"
    fi
fi

echo "serving the arbitrum-reth book at http://${host}:${port}  (ctrl-c to stop)"
exec mdbook serve "$book_dir" --hostname "$host" --port "$port" "${open_flag[@]}" "${args[@]}"
