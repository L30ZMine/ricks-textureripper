#!/usr/bin/env python3
"""Strip Rust comments from stdin and write the result to stdout.

Used as a git **clean** filter (see ``.gitattributes`` + ``tools/setup-git-filters``)
so the copy of ``*.rs`` stored in the repository / pushed to GitHub has no
comments, while the working tree keeps them.

It is a small Rust-aware scanner rather than a regex, so it never strips ``//``
or ``/* */`` that appear inside string, byte-string, raw-string, or char
literals, and it handles nested block comments and lifetimes. Correctness
matters: the output is what gets committed, so this must only ever remove
comments — never code.
"""

import sys


def _is_ident(c: str) -> bool:
    return c.isalnum() or c == "_"


def strip(src: str) -> str:
    out = []
    i = 0
    n = len(src)

    while i < n:
        c = src[i]
        # A raw-string / string / char prefix is only a prefix at a token
        # boundary (the preceding char isn't part of an identifier), so e.g. the
        # `r` in `foor` or `bar` is treated as ordinary code.
        boundary = (i == 0) or (not _is_ident(src[i - 1]))

        # --- Raw string: (b)r #* " ... " #* -------------------------------
        if boundary and (c == "r" or c == "b"):
            j = i
            if src[j] == "b":
                j += 1
            if j < n and src[j] == "r":
                k = j + 1
                hashes = 0
                while k < n and src[k] == "#":
                    hashes += 1
                    k += 1
                if k < n and src[k] == '"':
                    closer = '"' + "#" * hashes
                    end = src.find(closer, k + 1)
                    end = n if end == -1 else end + len(closer)
                    out.append(src[i:end])
                    i = end
                    continue
            # Not actually a raw string — fall through and emit normally.

        # --- Normal / byte string: (b)" ... " -----------------------------
        if c == '"' or (boundary and c == "b" and i + 1 < n and src[i + 1] == '"'):
            if c == "b":
                out.append("b")
                i += 1
            buf = ['"']
            i += 1  # skip the opening quote
            while i < n:
                ch = src[i]
                buf.append(ch)
                if ch == "\\":  # escape: take the next char verbatim too
                    if i + 1 < n:
                        buf.append(src[i + 1])
                        i += 2
                        continue
                    i += 1
                    break
                i += 1
                if ch == '"':
                    break
            out.append("".join(buf))
            continue

        # --- Char / byte-char literal vs. lifetime ------------------------
        if c == "'" or (boundary and c == "b" and i + 1 < n and src[i + 1] == "'"):
            pre = ""
            if c == "b":
                pre = "b"
                i += 1
            # src[i] == "'" here.
            if i + 1 < n and src[i + 1] == "\\":
                # Escaped char literal, e.g. '\n', '\\', '\'', '\u{1F600}'.
                j = i + 1
                while j < n:
                    if src[j] == "\\":
                        j += 2
                        continue
                    if src[j] == "'":
                        j += 1
                        break
                    j += 1
                out.append(pre + src[i:j])
                i = j
                continue
            if i + 2 < n and src[i + 2] == "'":
                # Simple char literal, e.g. 'a'.
                out.append(pre + src[i : i + 3])
                i += 3
                continue
            # Otherwise it's a lifetime ('a, 'static, '_) or a label — emit the
            # quote and carry on lexing the identifier as ordinary code.
            out.append(pre + "'")
            i += 1
            continue

        # --- Line comment: // ... (to end of line) ------------------------
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            j = i + 2
            while j < n and src[j] != "\n":
                j += 1
            i = j  # keep the newline; drop the comment text
            continue

        # --- Block comment: /* ... */ (nested) ----------------------------
        if c == "/" and i + 1 < n and src[i + 1] == "*":
            depth = 1
            j = i + 2
            while j < n and depth > 0:
                if src[j] == "/" and j + 1 < n and src[j + 1] == "*":
                    depth += 1
                    j += 2
                elif src[j] == "*" and j + 1 < n and src[j + 1] == "/":
                    depth -= 1
                    j += 2
                else:
                    j += 1
            i = j
            continue

        out.append(c)
        i += 1

    return "".join(out)


def cleanup(text: str) -> str:
    """Tidy the leftovers: trim trailing whitespace, collapse the blank lines a
    removed comment leaves behind, and end with a single newline."""
    lines = [ln.rstrip() for ln in text.split("\n")]
    result = []
    prev_blank = False
    for ln in lines:
        blank = ln == ""
        if blank and prev_blank:
            continue
        result.append(ln)
        prev_blank = blank
    while result and result[0] == "":
        result.pop(0)
    while result and result[-1] == "":
        result.pop()
    return ("\n".join(result) + "\n") if result else ""


def main() -> None:
    data = sys.stdin.buffer.read().decode("utf-8")
    sys.stdout.buffer.write(cleanup(strip(data)).encode("utf-8"))


if __name__ == "__main__":
    main()
