#!/usr/bin/env bash
set -e

echo "=== opn Navigator Hook Setup ==="

# 1. Ensure broot is installed
if ! command -v broot >/dev/null 2>&1; then
    echo "broot is not installed. Installing via cargo..."
    cargo install broot
else
    echo "broot is already installed."
fi

# 2. Ensure yazi is installed
if ! command -v yazi >/dev/null 2>&1; then
    echo "yazi is not installed. Installing via cargo..."
    cargo install --locked yazi-fm yazi-cli
else
    echo "yazi is already installed."
fi

# 3. Hook opn into broot
echo "Configuring broot to use opn..."
BROOT_CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/broot"
mkdir -p "$BROOT_CONF_DIR"

cat << 'EOF' > "$BROOT_CONF_DIR/opn-hook.hjson"
{
  verbs: [
    {
      key: enter
      apply_to: file
      # Spawns a new terminal window for opn using $TERMINAL
      external: ["sh", "-c", "${TERMINAL:-ghostty} -e opn \"$1\"", "broot-open", "{file}"]
      leave_broot: false
    }
  ]
}
EOF

BROOT_CONF="$BROOT_CONF_DIR/conf.hjson"
if [ ! -f "$BROOT_CONF" ]; then
    echo "imports: [ opn-hook.hjson ]" > "$BROOT_CONF"
elif ! grep -q "opn-hook.hjson" "$BROOT_CONF"; then
    if grep -q "^imports: \[" "$BROOT_CONF"; then
        sed -i '/^imports: \[/a \    opn-hook.hjson' "$BROOT_CONF"
    else
        echo -e "\nimports: [\n    opn-hook.hjson\n]" >> "$BROOT_CONF"
    fi
fi
echo "Successfully hooked opn into broot."

# 4. Hook opn into yazi via shell wrapper
echo "Configuring yazi shell wrapper..."

WRAPPER_FUNC=$(cat << 'EOF'

# opn + yazi wrapper (drops into opn upon selection)
function yazi-opn() {
	local tmp="$(mktemp -t "yazi-cwd.XXXXXX")"
	local choose="$(mktemp -t "yazi-choose.XXXXXX")"
	
	yazi "$@" --cwd-file="$tmp" --chooser-file="$choose"
	
	if cwd="$(cat -- "$tmp")" && [ -n "$cwd" ] && [ "$cwd" != "$PWD" ]; then
		builtin cd -- "$cwd"
	fi
	rm -f -- "$tmp"
	
	if [ -s "$choose" ]; then
		local file="$(cat -- "$choose")"
		rm -f -- "$choose"
		opn "$file"
	else
		rm -f -- "$choose"
	fi
}
alias yo='yazi-opn'
EOF
)

for RC_FILE in "$HOME/.zshrc" "$HOME/.bashrc"; do
    if [ -f "$RC_FILE" ]; then
        if ! grep -q "yazi-opn" "$RC_FILE"; then
            echo "$WRAPPER_FUNC" >> "$RC_FILE"
            echo "Added 'yo' alias to $RC_FILE"
        else
            echo "Yazi wrapper already exists in $RC_FILE"
        fi
    fi
done

echo ""
echo "Setup complete! Please restart your terminal or run:"
echo "  source ~/.zshrc (or ~/.bashrc)"
echo "You can now use 'yo' to launch yazi. Pressing enter on a file will drop you into opn."
echo "Broot will automatically open files in a new terminal window using opn."
