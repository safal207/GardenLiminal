#!/usr/bin/env bash
# GardenLiminal × LiminalDB — Integration Demo
#
# Shows the full flow:
#   1. Start LiminalDB (liminal-cli)
#   2. Run a container via GardenLiminal with --store liminal
#   3. Query the container's lifecycle history via LQL
#
# Requirements:
#   - GardenLiminal binary: ./target/release/gl
#   - LiminalDB binary:     path set in LIMINAL_BIN (or 'liminal-cli' on PATH)
#   - websocat (for LQL queries): https://github.com/vi/websocat

set -euo pipefail

GL="${GL_BIN:-./target/release/gl}"
LIMINAL="${LIMINAL_BIN:-liminal-cli}"
LIMINAL_URL="${LIMINAL_URL:-ws://127.0.0.1:8787}"
SEED="${1:-./examples/seed-busybox.yaml}"

# ── colours ──────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; BLUE='\033[0;34m'; YELLOW='\033[1;33m'; NC='\033[0m'
info()  { echo -e "${BLUE}[demo]${NC} $*"; }
ok()    { echo -e "${GREEN}[ok]${NC}   $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC} $*"; }

# ── 1. Build GardenLiminal ────────────────────────────────────────────────────
info "Building GardenLiminal..."
cargo build --release --quiet
ok "Binary ready: $GL"

# ── 2. Start LiminalDB ────────────────────────────────────────────────────────
if pgrep -x liminal-cli >/dev/null 2>&1; then
    warn "LiminalDB already running"
else
    info "Starting LiminalDB on port 8787..."
    "$LIMINAL" &
    LIMINAL_PID=$!
    trap "kill $LIMINAL_PID 2>/dev/null || true" EXIT
    sleep 1          # give it a moment to bind
    ok "LiminalDB started (PID $LIMINAL_PID)"
fi

# ── 3. Run container → events go to LiminalDB ─────────────────────────────────
info "Running container: $SEED"
info "Events will be sent to LiminalDB at $LIMINAL_URL"
echo ""

LIMINAL_URL="$LIMINAL_URL" sudo -E \
    "$GL" run -f "$SEED" --store liminal
echo ""

# ── 4. Query event history via LQL ───────────────────────────────────────────
if command -v websocat >/dev/null 2>&1; then
    info "Querying LiminalDB for recent container events..."
    echo ""

    # Send an LQL query and read the first response
    echo '{"cmd":"lql","q":"SELECT * WHERE type = EVENT LIMIT 20"}' \
        | websocat --no-close -n1 "$LIMINAL_URL" \
        | python3 -m json.tool 2>/dev/null || true

    echo ""
    ok "Done. All container lifecycle events are stored in LiminalDB."
else
    warn "websocat not found — skipping LQL query step"
    warn "Install: cargo install websocat"
    echo ""
    info "To query events manually:"
    echo "  echo '{\"cmd\":\"lql\",\"q\":\"SELECT * WHERE type = EVENT LIMIT 20\"}' \\"
    echo "    | websocat -n1 $LIMINAL_URL"
fi

echo ""
info "To explore more:"
echo "  # Subscribe to live events"
echo "  echo '{\"cmd\":\"subscribe\",\"pattern\":\"gl/*\"}' | websocat $LIMINAL_URL"
echo ""
echo "  # Mirror timeline (replay history)"
echo "  echo '{\"cmd\":\"mirror.timeline\",\"top\":50}' | websocat -n1 $LIMINAL_URL"
