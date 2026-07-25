{ pkgs, guiRoot }:
# wraps `dx fmt` and `rustywind` into a single treefmt formatter for *.rs.
#
# `dx fmt -c` (check mode) is broken as of dioxus-cli 0.8.0-alpha.0: it
# rewrites the file and exits 0 regardless of whether formatting was
# needed, which busts treefmt's mtime/size cache and trips
# `treefmt --fail-on-change` on already-formatted files. `dx fmt -f -`
# (stdin/stdout) doesn't have this problem, so we format into a temp
# file and only copy it back over the original if the contents
# actually changed.
#
# rustywind also needs help: its default class-sorting regex only
# matches html-style `class="..."` attributes, not dioxus's
# `class: "..."` rsx syntax, so it's a silent no-op on this codebase
# without `--custom-regex`. and its *built-in* sort order predates
# tailwind v4 and daisyui, so `--output-css-file` is used to derive the
# real order from a one-off tailwind build instead.
pkgs.writeShellApplication {
  name = "dx-fmt";
  runtimeInputs = with pkgs; [
    coreutils
    diffutils
    dioxus-cli
    rustywind
    tailwindcss_4
  ];
  text = ''
    # build the gui crate's tailwind output once per invocation so rustywind
    # can sort classes against the project's actual utility order (including
    # daisyui) instead of falling back to its bundled tailwind v3-era order.
    css_output="$(mktemp --suffix=.css)"
    trap 'rm -f "$css_output"' EXIT

    tailwindcss \
      --input "${guiRoot}/tailwind.css" \
      --output "$css_output" \
      --cwd "${guiRoot}" \
      --silent

    status=0

    for file in "$@"; do
      formatted="$(mktemp --suffix=.rs)"

      if ! dx fmt -f - <"$file" >"$formatted"; then
        echo "dx-fmt: warning: couldn't parse '$file' as rsx, leaving it as-is" >&2
        rm -f "$formatted"
        status=1
        continue
      fi

      rustywind \
        --quiet \
        --write \
        --custom-regex 'class:\s*"([^"]*)"' \
        --output-css-file "$css_output" \
        "$formatted"

      cmp -s "$file" "$formatted" || cp "$formatted" "$file"
      rm -f "$formatted"
    done

    exit "$status"
  '';
}
