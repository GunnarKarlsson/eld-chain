#!/bin/bash

# Kill any existing eld node processes
echo "Checking for existing eld node processes..."
# Find processes that might be the eld node app
# Look for cargo run processes in this directory, or processes listening on port 26658 (ABCI)
EXISTING_PIDS=$(lsof -ti:26658 2>/dev/null)
if [ -n "$EXISTING_PIDS" ]; then
    echo "Found existing processes on port 26658, killing them..."
    kill -TERM $EXISTING_PIDS 2>/dev/null
    sleep 2
    kill -KILL $EXISTING_PIDS 2>/dev/null
fi

# Also check for cargo processes running in this directory
CARGO_PIDS=$(ps aux | grep -E "cargo.*run|eld_node_app" | grep -v grep | awk '{print $2}')
if [ -n "$CARGO_PIDS" ]; then
    echo "Found existing cargo/eld processes, killing them..."
    kill -TERM $CARGO_PIDS 2>/dev/null
    sleep 2
    kill -KILL $CARGO_PIDS 2>/dev/null
fi

# Wait a moment for ports to be released
if [ -n "$EXISTING_PIDS" ] || [ -n "$CARGO_PIDS" ]; then
    echo "Waiting for ports to be released..."
    sleep 2
fi

# Set up transaction response logging
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TX_RESPONSES_DIR="$SCRIPT_DIR/tx_responses"
mkdir -p "$TX_RESPONSES_DIR"
# Clear old log files from previous runs
rm -f "$TX_RESPONSES_DIR"/*.txt 2>/dev/null

# Function to write transaction response to file
# Usage: write_tx_response "transaction_type" "response_output"
write_tx_response() {
    local tx_type="$1"
    local response="$2"
    local timestamp=$(date +"%Y-%m-%d_%H-%M-%S")
    local filename="${timestamp}_${tx_type}.txt"
    local filepath="$TX_RESPONSES_DIR/$filename"
    
    # Write response to file
    {
        echo "=========================================="
        echo "Transaction Type: $tx_type"
        echo "Timestamp: $(date)"
        echo "=========================================="
        echo ""
        echo "$response"
        echo ""
        echo "=========================================="
    } > "$filepath"
    
    # Ensure write is complete before continuing
    sync
    
    echo "Transaction response written to: $filepath"
}

# Function to poll account nonce until it matches expected value
# Usage: wait_for_nonce_update "account_address" "expected_nonce" [max_wait_seconds]
# Returns 0 if nonce matches, 1 if timeout
wait_for_nonce_update() {
    local account_address="$1"
    local expected_nonce="$2"
    local max_wait="${3:-30}"  # Default 30 seconds
    local check_interval=1     # Check every 1 second
    local elapsed=0
    
    echo "Waiting for account $account_address nonce to reach $expected_nonce..."
    
    while [ $elapsed -lt $max_wait ]; do
        # Query the account
        local account_output=$(cargo run -- get-account "$account_address" 2>&1)
        local exit_code=$?
        
        if [ $exit_code -ne 0 ]; then
            echo "Warning: Failed to query account, retrying in ${check_interval}s..."
            sleep $check_interval
            elapsed=$((elapsed + check_interval))
            continue
        fi
        
        # Extract nonce from output
        # Format: "Account { address: ..., balance: ... units, nonce: <number> }"
        local current_nonce=""
        
        # Try multiple patterns to extract nonce
        # Pattern 1: "nonce: " followed by digits
        current_nonce=$(echo "$account_output" | grep -oE "nonce:[[:space:]]+[0-9]+" | grep -oE "[0-9]+" | head -1)
        
        # Pattern 2: "nonce:" followed by optional space and digits
        if [ -z "$current_nonce" ]; then
            current_nonce=$(echo "$account_output" | grep -oE "nonce:[[:space:]]*[0-9]+" | grep -oE "[0-9]+" | head -1)
        fi
        
        # Pattern 3: JSON format "nonce": number
        if [ -z "$current_nonce" ]; then
            current_nonce=$(echo "$account_output" | grep -oE "\"nonce\"[[:space:]]*:[[:space:]]*[0-9]+" | grep -oE "[0-9]+" | head -1)
        fi
        
        # Pattern 4: Using jq if available
        if [ -z "$current_nonce" ] && command -v jq &> /dev/null; then
            current_nonce=$(echo "$account_output" | jq -r '.nonce // empty' 2>/dev/null)
        fi
        
        if [ -n "$current_nonce" ] && [ "$current_nonce" = "$expected_nonce" ]; then
            echo "SUCCESS: Account nonce has reached expected value: $expected_nonce"
            return 0
        fi
        
        if [ -n "$current_nonce" ]; then
            echo "Current nonce: $current_nonce, expected: $expected_nonce (waiting ${check_interval}s...)"
        else
            echo "Could not extract nonce from account output, retrying in ${check_interval}s..."
        fi
        
        sleep $check_interval
        elapsed=$((elapsed + check_interval))
    done
    
    echo "ERROR: Timeout waiting for account nonce to reach $expected_nonce after ${max_wait}s"
    return 1
}

# Development wallet configuration
# Using wallet2 to avoid conflicts with validator wallet (wallet1)
DEV_WALLET="wallet2"
# Single capacity-validator wallet (local slots + RegisterCapacity + VerifiedProof)
export ELD_CAPACITY_VALIDATOR_WALLET_NAME=wallet-capacity-validator-1

# Delete DB files and start the app
sh ./scripts/delete-db-files.sh

# Remove all JSON files from contract-deployments folder
echo "Removing contract deployment JSON files..."
rm -f ../../clients/cli/contract-deployments/*.json
if [ $? -eq 0 ]; then
    echo "Contract deployment JSON files removed successfully"
else
    echo "Warning: Failed to remove some contract deployment JSON files (folder may not exist or be empty)"
fi

# Detect which terminal application is being used
if [ -n "$TERM_PROGRAM" ]; then
    TERMINAL_APP="$TERM_PROGRAM"
else
    # Try to detect Terminal.app or iTerm2
    if ps aux | grep -i "iTerm" | grep -v grep > /dev/null; then
        TERMINAL_APP="iTerm"
    else
        TERMINAL_APP="Terminal"
    fi
fi

# Get the current directory for the new tab
CURRENT_DIR=$(pwd)

# Start the eld node app in a new terminal tab
echo "Starting eld node app in a new terminal tab..."
if [ "$TERMINAL_APP" = "iTerm.app" ] || [ "$TERMINAL_APP" = "iTerm" ]; then
    # iTerm2
    osascript -e "tell application \"iTerm\"
        tell current window
            create tab with default profile
            tell current session of current tab
                write text \"cd '$CURRENT_DIR' && export ELD_CAPACITY_VALIDATOR_WALLET_NAME='$ELD_CAPACITY_VALIDATOR_WALLET_NAME' && echo 'Starting Eld Node App...' && cargo run\"
            end tell
        end tell
    end tell" 2>/dev/null
    if [ $? -eq 0 ]; then
        echo "Eld node app started in new iTerm2 tab"
        APP_PID=""  # We can't track PID from another terminal
    else
        echo "Failed to open iTerm2 tab, falling back to background process"
        export ELD_CAPACITY_VALIDATOR_WALLET_NAME="$ELD_CAPACITY_VALIDATOR_WALLET_NAME"
        cargo run &
        APP_PID=$!
    fi
else
    # Terminal.app (default macOS terminal)
    osascript -e "tell application \"Terminal\"
        activate
        tell application \"System Events\" to keystroke \"t\" using command down
        delay 0.3
        do script \"cd '$CURRENT_DIR' && export ELD_CAPACITY_VALIDATOR_WALLET_NAME='$ELD_CAPACITY_VALIDATOR_WALLET_NAME' && echo 'Starting Eld Node App...' && cargo run\" in front window
    end tell" 2>/dev/null
    if [ $? -eq 0 ]; then
        echo "Eld node app started in new Terminal tab"
        APP_PID=""  # We can't track PID from another terminal
    else
        echo "Failed to open Terminal tab, falling back to background process"
        export ELD_CAPACITY_VALIDATOR_WALLET_NAME="$ELD_CAPACITY_VALIDATOR_WALLET_NAME"
        cargo run &
        APP_PID=$!
    fi
fi

# Wait 1 second for eld node app to start, then start Tendermint in a new terminal tab
sleep 1
echo "Starting Tendermint node in a new terminal tab..."

# Start Tendermint in a new terminal tab based on the terminal application
if [ "$TERMINAL_APP" = "iTerm.app" ] || [ "$TERMINAL_APP" = "iTerm" ]; then
    # iTerm2
    osascript -e "tell application \"iTerm\"
        tell current window
            create tab with default profile
            tell current session of current tab
                write text \"cd '$CURRENT_DIR' && echo 'Starting Tendermint node...' && tendermint unsafe_reset_all && tendermint node --proxy_app=tcp://127.0.0.1:26658 --log_level debug\"
            end tell
        end tell
    end tell" 2>/dev/null
    if [ $? -eq 0 ]; then
        echo "Tendermint started in new iTerm2 tab"
        TM_PID=""  # We can't track PID from another terminal
    else
        echo "Failed to open iTerm2 tab, falling back to background process"
        tendermint unsafe_reset_all && tendermint node --proxy_app=tcp://127.0.0.1:26658 --log_level debug &
        TM_PID=$!
    fi
else
    # Terminal.app (default macOS terminal)
    osascript -e "tell application \"Terminal\"
        activate
        tell application \"System Events\" to keystroke \"t\" using command down
        delay 0.3
        do script \"cd '$CURRENT_DIR' && echo 'Starting Tendermint node...' && tendermint unsafe_reset_all && tendermint node --proxy_app=tcp://127.0.0.1:26658 --log_level debug\" in front window
    end tell" 2>/dev/null
    if [ $? -eq 0 ]; then
        echo "Tendermint started in new Terminal tab"
        TM_PID=""  # We can't track PID from another terminal
    else
        echo "Failed to open Terminal tab, falling back to background process"
        tendermint unsafe_reset_all && tendermint node --proxy_app=tcp://127.0.0.1:26658 --log_level debug &
        TM_PID=$!
    fi
fi

# Function to cleanup on exit - kills the process and all its children
cleanup() {
    echo ""
    echo "Shutting down processes..."
    
    # Kill Tendermint node (if running in background)
    if [ -n "$TM_PID" ] && kill -0 $TM_PID 2>/dev/null; then
        echo "Shutting down Tendermint (PID: $TM_PID)..."
        kill -TERM $TM_PID 2>/dev/null
        sleep 1
        kill -KILL $TM_PID 2>/dev/null
    else
        # If Tendermint is running in another terminal, try to find and kill it
        echo "Shutting down Tendermint node..."
        pkill -f "tendermint node --proxy_app" 2>/dev/null
        sleep 1
        pkill -9 -f "tendermint node --proxy_app" 2>/dev/null
    fi
    
    # Kill eld node app
    if [ -n "$APP_PID" ] && kill -0 $APP_PID 2>/dev/null; then
        # Running in background with known PID
    echo "Shutting down eld node app (PID: $APP_PID)..."
    # Get the process group ID
    PGID=$(ps -o pgid= -p $APP_PID 2>/dev/null | tr -d ' ')
    if [ -n "$PGID" ]; then
        # Kill the entire process group (works on both Linux and macOS)
        kill -TERM -$PGID 2>/dev/null
        sleep 2
        kill -KILL -$PGID 2>/dev/null
    else
        # Fallback: kill the process and try to find children
        kill -TERM $APP_PID 2>/dev/null
        # Kill any child processes (cargo, rust processes, etc.)
        pkill -P $APP_PID 2>/dev/null
        sleep 2
        kill -KILL $APP_PID 2>/dev/null
        pkill -9 -P $APP_PID 2>/dev/null
    fi
    wait $APP_PID 2>/dev/null
    else
        # Running in another terminal, try to find and kill it
        echo "Shutting down eld node app..."
        pkill -f "cargo run" 2>/dev/null
        pkill -f "eld_node_app" 2>/dev/null
        # Also kill any rust processes that might be the node app
        pkill -f "target/debug/eld_node_app" 2>/dev/null
        pkill -f "target/release/eld_node_app" 2>/dev/null
        sleep 1
        pkill -9 -f "cargo run" 2>/dev/null
        pkill -9 -f "eld_node_app" 2>/dev/null
        pkill -9 -f "target/debug/eld_node_app" 2>/dev/null
        pkill -9 -f "target/release/eld_node_app" 2>/dev/null
    fi
    wait $TM_PID 2>/dev/null
    exit 0
}

# Set up signal handlers to forward Ctrl+C to the app process
trap cleanup SIGINT SIGTERM

# Function to check if app is ready
check_app_ready() {
    local response=$(curl -s http://localhost:26657/status 2>/dev/null)
    if [ $? -ne 0 ]; then
        return 1
    fi
    
    # Extract latest_block_height from JSON response
    # Using jq if available, otherwise use grep/sed fallback
    if command -v jq &> /dev/null; then
        local block_height=$(echo "$response" | jq -r '.result.sync_info.latest_block_height // "0"')
    else
        # Fallback: extract block height using grep/sed
        # The value is a string in JSON, so we extract the quoted value
        local block_height=$(echo "$response" | grep -o '"latest_block_height"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"latest_block_height"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
        # If extraction failed, default to 0
        if [ -z "$block_height" ]; then
            block_height="0"
        fi
    fi
    
    # Check if block_height is a number and greater than 0
    if [[ "$block_height" =~ ^[0-9]+$ ]] && [ "$block_height" -gt 0 ]; then
        return 0
    else
        return 1
    fi
}

# Wait for app to be ready
echo "Waiting for app to be ready..."
MAX_WAIT=300  # 5 minutes max wait
WAIT_TIME=0
CHECK_INTERVAL=2

while [ $WAIT_TIME -lt $MAX_WAIT ]; do
    if check_app_ready; then
        echo "App is ready! Block height > 0 detected."
        break
    fi
    
    # Check if the app process is still running (only if we have a PID)
    if [ -n "$APP_PID" ] && ! kill -0 $APP_PID 2>/dev/null; then
        echo "Error: App process died unexpectedly"
        exit 1
    fi
    
    # Check if Tendermint process is still running (only if we have a PID)
    if [ -n "$TM_PID" ] && ! kill -0 $TM_PID 2>/dev/null; then
        echo "Warning: Tendermint process died unexpectedly"
    fi
    
    sleep $CHECK_INTERVAL
    WAIT_TIME=$((WAIT_TIME + CHECK_INTERVAL))
    echo "Still waiting... (${WAIT_TIME}s)"
done

if [ $WAIT_TIME -ge $MAX_WAIT ]; then
    echo "Error: App did not become ready within ${MAX_WAIT} seconds"
    if [ -n "$APP_PID" ]; then
    kill $APP_PID 2>/dev/null
    fi
    exit 1
fi

# Deploy eSOL token contract
echo "Deploying eSOL token contract..."
cd ../../clients/cli || exit 1
ESOL_DEPLOY_OUTPUT=$(cargo run -- add-contract "$DEV_WALLET" "contract_binaries/token_contract.wasm" "./contract_init_args/esol_token_init_args.json" 2>&1)
ESOL_DEPLOY_EXIT_CODE=$?
echo "$ESOL_DEPLOY_OUTPUT"
write_tx_response "deploy_esol_token" "$ESOL_DEPLOY_OUTPUT"
if [ $ESOL_DEPLOY_EXIT_CODE -ne 0 ]; then
    echo "ERROR: eSOL token contract deployment failed"
    exit 1
fi

# Deploy STAR token contract
echo "Deploying STAR token contract..."
STAR_DEPLOY_OUTPUT=$(cargo run -- add-contract "$DEV_WALLET" "contract_binaries/token_contract.wasm" "./contract_init_args/star_token_init_args.json" 2>&1)
STAR_DEPLOY_EXIT_CODE=$?
echo "$STAR_DEPLOY_OUTPUT"
write_tx_response "deploy_star_token" "$STAR_DEPLOY_OUTPUT"
if [ $STAR_DEPLOY_EXIT_CODE -ne 0 ]; then
    echo "ERROR: STAR token contract deployment failed"
    exit 1
fi

# Wait a moment for STAR deployment to complete and be written to disk
sleep 2

# Extract eSOL contract ID from deployment file (needed for BasicReceiver init args)
ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER=""
ESOL_DEPLOYMENT_FILE_FOR_BASIC_RECEIVER=""

# Try multiple possible filename patterns
POSSIBLE_ESOL_PATTERNS_FOR_BASIC_RECEIVER=(
    "esol-token-contract-*.json"
    "*esol*.json"
)

for pattern in "${POSSIBLE_ESOL_PATTERNS_FOR_BASIC_RECEIVER[@]}"; do
    if command -v jq &> /dev/null; then
        ESOL_DEPLOYMENT_FILE_FOR_BASIC_RECEIVER=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$ESOL_DEPLOYMENT_FILE_FOR_BASIC_RECEIVER" ] && [ -f "$ESOL_DEPLOYMENT_FILE_FOR_BASIC_RECEIVER" ]; then
            ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER=$(jq -r '.contract_id // empty' "$ESOL_DEPLOYMENT_FILE_FOR_BASIC_RECEIVER" 2>/dev/null)
            if [ -n "$ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER" ] && [ "$ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER" != "null" ] && [ "$ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER" != "empty" ]; then
                break
            fi
        fi
    else
        # Fallback: try to extract using grep/sed
        ESOL_DEPLOYMENT_FILE_FOR_BASIC_RECEIVER=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$ESOL_DEPLOYMENT_FILE_FOR_BASIC_RECEIVER" ] && [ -f "$ESOL_DEPLOYMENT_FILE_FOR_BASIC_RECEIVER" ]; then
            ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER=$(grep -o '"contract_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$ESOL_DEPLOYMENT_FILE_FOR_BASIC_RECEIVER" 2>/dev/null | sed -E 's/.*"contract_id"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
            if [ -n "$ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER" ] && [ "$ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER" != "null" ]; then
                break
            fi
        fi
    fi
done

# Verify eSOL contract ID was extracted
if [ -z "$ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER" ] || [ "$ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER" = "null" ]; then
    echo ""
    echo "ERROR: Could not extract eSOL contract_id from deployment file (needed for BasicReceiver)"
    echo "Current directory: $(pwd)"
    echo "Looking in: contract-deployments/"
    if [ -d "contract-deployments" ]; then
        echo "Files in contract-deployments/:"
        ls -la contract-deployments/ 2>/dev/null || echo "  (directory exists but listing failed)"
    else
        echo "ERROR: contract-deployments/ directory does not exist!"
    fi
    echo "Cannot proceed with BasicReceiver deployment without eSOL contract ID."
    exit 1
fi

echo "eSOL contract ID extracted for BasicReceiver: $ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER"

# Validate that basic_receiver_init_args.json has the correct eSOL contract ID
echo "Validating BasicReceiver init args match deployed eSOL contract..."
BASIC_RECEIVER_ESOL_TOKEN=""
if command -v jq &> /dev/null; then
    BASIC_RECEIVER_ESOL_TOKEN=$(jq -r '.args.esol_token // empty' "./contract_init_args/basic_receiver_init_args.json" 2>/dev/null)
else
    BASIC_RECEIVER_ESOL_TOKEN=$(grep -o '"esol_token"[[:space:]]*:[[:space:]]*"[^"]*"' "./contract_init_args/basic_receiver_init_args.json" 2>/dev/null | sed -E 's/.*"esol_token"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
fi

if [ -z "$BASIC_RECEIVER_ESOL_TOKEN" ] || [ "$BASIC_RECEIVER_ESOL_TOKEN" = "null" ]; then
    echo "ERROR: Could not extract esol_token from basic_receiver_init_args.json"
    exit 1
fi

# Normalize addresses (remove 0x prefix for comparison if needed)
ESOL_FOR_COMPARE=$(echo "$ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER" | sed 's/^0x//')
BASIC_RECEIVER_ESOL_FOR_COMPARE=$(echo "$BASIC_RECEIVER_ESOL_TOKEN" | sed 's/^0x//')

if [ "$ESOL_FOR_COMPARE" != "$BASIC_RECEIVER_ESOL_FOR_COMPARE" ]; then
    echo "ERROR: eSOL contract ID mismatch!"
    echo "  Deployed eSOL contract ID: $ESOL_CONTRACT_ID_FOR_BASIC_RECEIVER"
    echo "  BasicReceiver init args esol_token: $BASIC_RECEIVER_ESOL_TOKEN"
    echo "  These must match! Update basic_receiver_init_args.json with the correct eSOL contract ID."
    exit 1
fi

echo "✓ BasicReceiver init args validation passed: eSOL contract ID matches"

# Deploy BasicReceiver contract with original init args file
echo "Deploying BasicReceiver contract..."
BASIC_RECEIVER_DEPLOY_OUTPUT=$(cargo run -- add-contract "$DEV_WALLET" "contract_binaries/basic_receiver_contract.wasm" "./contract_init_args/basic_receiver_init_args.json" 2>&1)
BASIC_RECEIVER_DEPLOY_EXIT_CODE=$?
echo "$BASIC_RECEIVER_DEPLOY_OUTPUT"
write_tx_response "deploy_basic_receiver" "$BASIC_RECEIVER_DEPLOY_OUTPUT"
if [ $BASIC_RECEIVER_DEPLOY_EXIT_CODE -ne 0 ]; then
    echo "ERROR: BasicReceiver contract deployment failed (command returned non-zero exit code)"
    exit 1
fi

# Wait a moment for the deployment to complete and be written to disk
# Also allows time for nonce updates to propagate
sleep 3

# Verify BasicReceiver deployment file was created
BASIC_RECEIVER_DEPLOYMENT_FILE=""
POSSIBLE_PATTERNS=(
    "basic-receiver-contract-*.json"
    "basic_receiver-contract-*.json"
    "*basic*receiver*.json"
    "*receiver*.json"
)

for pattern in "${POSSIBLE_PATTERNS[@]}"; do
    BASIC_RECEIVER_DEPLOYMENT_FILE=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
    if [ -n "$BASIC_RECEIVER_DEPLOYMENT_FILE" ] && [ -f "$BASIC_RECEIVER_DEPLOYMENT_FILE" ]; then
        break
    fi
done

# If still not found, try searching all JSON files
if [ -z "$BASIC_RECEIVER_DEPLOYMENT_FILE" ] || [ ! -f "$BASIC_RECEIVER_DEPLOYMENT_FILE" ]; then
    ALL_DEPLOYMENT_FILES=$(ls -t contract-deployments/*.json 2>/dev/null)
    if [ -n "$ALL_DEPLOYMENT_FILES" ]; then
        for file in $ALL_DEPLOYMENT_FILES; do
            filename=$(basename "$file")
            # Skip known contract files
            if [[ "$filename" != *"esol-token"* ]] && [[ "$filename" != *"star-token"* ]] && [[ "$filename" != *"amm"* ]]; then
                BASIC_RECEIVER_DEPLOYMENT_FILE="$file"
                break
            fi
        done
    fi
fi

# Verify BasicReceiver deployment file exists
if [ -z "$BASIC_RECEIVER_DEPLOYMENT_FILE" ] || [ ! -f "$BASIC_RECEIVER_DEPLOYMENT_FILE" ]; then
    echo ""
    echo "ERROR: BasicReceiver contract deployment file was not created"
    echo "Current directory: $(pwd)"
    echo "Looking in: contract-deployments/"
    if [ -d "contract-deployments" ]; then
        echo "Files in contract-deployments/:"
        ls -la contract-deployments/ 2>/dev/null || echo "  (directory exists but listing failed)"
        echo ""
        echo "Expected file pattern: basic-receiver-contract-*.json"
        echo "The BasicReceiver contract deployment appears to have failed."
        echo "Check the deployment output above for error messages."
    else
        echo "ERROR: contract-deployments/ directory does not exist!"
    fi
    exit 1
fi

echo "BasicReceiver deployment file found: $BASIC_RECEIVER_DEPLOYMENT_FILE"

# Extract eSOL and STAR contract IDs for AMM init args (needed before AMM deployment)
ESOL_CONTRACT_ID_FOR_AMM=""
STAR_CONTRACT_ID_FOR_AMM=""

# Extract eSOL contract ID
POSSIBLE_ESOL_PATTERNS_FOR_AMM=(
    "esol-token-contract-*.json"
    "*esol*.json"
)

for pattern in "${POSSIBLE_ESOL_PATTERNS_FOR_AMM[@]}"; do
    if command -v jq &> /dev/null; then
        ESOL_DEPLOYMENT_FILE_FOR_AMM=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$ESOL_DEPLOYMENT_FILE_FOR_AMM" ] && [ -f "$ESOL_DEPLOYMENT_FILE_FOR_AMM" ]; then
            ESOL_CONTRACT_ID_FOR_AMM=$(jq -r '.contract_id // empty' "$ESOL_DEPLOYMENT_FILE_FOR_AMM" 2>/dev/null)
            if [ -n "$ESOL_CONTRACT_ID_FOR_AMM" ] && [ "$ESOL_CONTRACT_ID_FOR_AMM" != "null" ] && [ "$ESOL_CONTRACT_ID_FOR_AMM" != "empty" ]; then
                break
            fi
        fi
    else
        ESOL_DEPLOYMENT_FILE_FOR_AMM=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$ESOL_DEPLOYMENT_FILE_FOR_AMM" ] && [ -f "$ESOL_DEPLOYMENT_FILE_FOR_AMM" ]; then
            ESOL_CONTRACT_ID_FOR_AMM=$(grep -o '"contract_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$ESOL_DEPLOYMENT_FILE_FOR_AMM" 2>/dev/null | sed -E 's/.*"contract_id"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
            if [ -n "$ESOL_CONTRACT_ID_FOR_AMM" ] && [ "$ESOL_CONTRACT_ID_FOR_AMM" != "null" ]; then
                break
            fi
        fi
    fi
done

# Extract STAR contract ID
POSSIBLE_STAR_PATTERNS_FOR_AMM=(
    "star-token-contract-*.json"
    "*star*.json"
)

for pattern in "${POSSIBLE_STAR_PATTERNS_FOR_AMM[@]}"; do
    if command -v jq &> /dev/null; then
        STAR_DEPLOYMENT_FILE_FOR_AMM=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$STAR_DEPLOYMENT_FILE_FOR_AMM" ] && [ -f "$STAR_DEPLOYMENT_FILE_FOR_AMM" ]; then
            STAR_CONTRACT_ID_FOR_AMM=$(jq -r '.contract_id // empty' "$STAR_DEPLOYMENT_FILE_FOR_AMM" 2>/dev/null)
            if [ -n "$STAR_CONTRACT_ID_FOR_AMM" ] && [ "$STAR_CONTRACT_ID_FOR_AMM" != "null" ] && [ "$STAR_CONTRACT_ID_FOR_AMM" != "empty" ]; then
                break
            fi
        fi
    else
        STAR_DEPLOYMENT_FILE_FOR_AMM=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$STAR_DEPLOYMENT_FILE_FOR_AMM" ] && [ -f "$STAR_DEPLOYMENT_FILE_FOR_AMM" ]; then
            STAR_CONTRACT_ID_FOR_AMM=$(grep -o '"contract_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$STAR_DEPLOYMENT_FILE_FOR_AMM" 2>/dev/null | sed -E 's/.*"contract_id"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
            if [ -n "$STAR_CONTRACT_ID_FOR_AMM" ] && [ "$STAR_CONTRACT_ID_FOR_AMM" != "null" ]; then
                break
            fi
        fi
    fi
done

# Verify both contract IDs were extracted
if [ -z "$ESOL_CONTRACT_ID_FOR_AMM" ] || [ "$ESOL_CONTRACT_ID_FOR_AMM" = "null" ]; then
    echo ""
    echo "ERROR: Could not extract eSOL contract_id from deployment file (needed for AMM)"
    echo "Cannot proceed with AMM deployment without eSOL contract ID."
    exit 1
fi

if [ -z "$STAR_CONTRACT_ID_FOR_AMM" ] || [ "$STAR_CONTRACT_ID_FOR_AMM" = "null" ]; then
    echo ""
    echo "ERROR: Could not extract STAR contract_id from deployment file (needed for AMM)"
    echo "Cannot proceed with AMM deployment without STAR contract ID."
    exit 1
fi

echo "eSOL contract ID extracted for AMM: $ESOL_CONTRACT_ID_FOR_AMM"
echo "STAR contract ID extracted for AMM: $STAR_CONTRACT_ID_FOR_AMM"

# Validate that amm_init_args.json has the correct contract IDs
echo "Validating AMM init args match deployed token contracts..."
AMM_TOKEN1_CW20=""
AMM_TOKEN2_CW20=""
if command -v jq &> /dev/null; then
    AMM_TOKEN1_CW20=$(jq -r '.args.token1_denom.cw20 // empty' "./contract_init_args/amm_init_args.json" 2>/dev/null)
    AMM_TOKEN2_CW20=$(jq -r '.args.token2_denom.cw20 // empty' "./contract_init_args/amm_init_args.json" 2>/dev/null)
else
    # Fallback: extract using grep/sed
    AMM_TOKEN1_CW20=$(grep -o '"token1_denom"[[:space:]]*:[[:space:]]*{[^}]*"cw20"[[:space:]]*:[[:space:]]*"[^"]*"' "./contract_init_args/amm_init_args.json" 2>/dev/null | sed -E 's/.*"cw20"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
    AMM_TOKEN2_CW20=$(grep -o '"token2_denom"[[:space:]]*:[[:space:]]*{[^}]*"cw20"[[:space:]]*:[[:space:]]*"[^"]*"' "./contract_init_args/amm_init_args.json" 2>/dev/null | sed -E 's/.*"cw20"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
fi

if [ -z "$AMM_TOKEN1_CW20" ] || [ "$AMM_TOKEN1_CW20" = "null" ]; then
    echo "ERROR: Could not extract token1_denom.cw20 from amm_init_args.json"
    exit 1
fi

if [ -z "$AMM_TOKEN2_CW20" ] || [ "$AMM_TOKEN2_CW20" = "null" ]; then
    echo "ERROR: Could not extract token2_denom.cw20 from amm_init_args.json"
    exit 1
fi

# Normalize addresses (remove 0x prefix for comparison if needed)
ESOL_FOR_COMPARE=$(echo "$ESOL_CONTRACT_ID_FOR_AMM" | sed 's/^0x//')
STAR_FOR_COMPARE=$(echo "$STAR_CONTRACT_ID_FOR_AMM" | sed 's/^0x//')
AMM_TOKEN1_FOR_COMPARE=$(echo "$AMM_TOKEN1_CW20" | sed 's/^0x//')
AMM_TOKEN2_FOR_COMPARE=$(echo "$AMM_TOKEN2_CW20" | sed 's/^0x//')

# Check if token1 matches eSOL and token2 matches STAR
if [ "$ESOL_FOR_COMPARE" != "$AMM_TOKEN1_FOR_COMPARE" ]; then
    echo "ERROR: AMM token1_denom.cw20 does not match deployed eSOL contract ID!"
    echo "  Deployed eSOL contract ID: $ESOL_CONTRACT_ID_FOR_AMM"
    echo "  AMM init args token1_denom.cw20: $AMM_TOKEN1_CW20"
    echo "  These must match! Update amm_init_args.json with the correct eSOL contract ID."
    exit 1
fi

if [ "$STAR_FOR_COMPARE" != "$AMM_TOKEN2_FOR_COMPARE" ]; then
    echo "ERROR: AMM token2_denom.cw20 does not match deployed STAR contract ID!"
    echo "  Deployed STAR contract ID: $STAR_CONTRACT_ID_FOR_AMM"
    echo "  AMM init args token2_denom.cw20: $AMM_TOKEN2_CW20"
    echo "  These must match! Update amm_init_args.json with the correct STAR contract ID."
    exit 1
fi

echo "✓ AMM init args validation passed: token contract IDs match deployed contracts"

# Deploy AMM contract with original init args file
echo "Deploying AMM contract..."
AMM_DEPLOY_OUTPUT=$(cargo run -- add-contract "$DEV_WALLET" "contract_binaries/amm.wasm" "./contract_init_args/amm_init_args.json" 2>&1)
AMM_DEPLOY_EXIT_CODE=$?
echo "$AMM_DEPLOY_OUTPUT"
write_tx_response "deploy_amm" "$AMM_DEPLOY_OUTPUT"
if [ $AMM_DEPLOY_EXIT_CODE -ne 0 ]; then
    echo "ERROR: AMM contract deployment failed"
    exit 1
fi

# Wait a moment for the AMM deployment to complete and be written to disk
# Also allows time for nonce updates to propagate
sleep 3

# Verify we're in the correct directory
if [ ! -d "contract-deployments" ]; then
    echo "ERROR: contract-deployments/ directory not found in current directory: $(pwd)"
    echo "Expected to be in cli directory"
    exit 1
fi

# Extract contract IDs and state IDs from deployment files

# Extract eSOL contract_id from deployment file
ESOL_CONTRACT_ID=""
ESOL_DEPLOYMENT_FILE=""

# Try multiple possible filename patterns
POSSIBLE_ESOL_PATTERNS=(
    "esol-token-contract-*.json"
    "*esol*.json"
)

for pattern in "${POSSIBLE_ESOL_PATTERNS[@]}"; do
    if command -v jq &> /dev/null; then
        ESOL_DEPLOYMENT_FILE=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$ESOL_DEPLOYMENT_FILE" ] && [ -f "$ESOL_DEPLOYMENT_FILE" ]; then
            ESOL_CONTRACT_ID=$(jq -r '.contract_id // empty' "$ESOL_DEPLOYMENT_FILE" 2>/dev/null)
            if [ -n "$ESOL_CONTRACT_ID" ] && [ "$ESOL_CONTRACT_ID" != "null" ] && [ "$ESOL_CONTRACT_ID" != "empty" ]; then
                break
            fi
        fi
    else
        # Fallback: try to extract using grep/sed
        ESOL_DEPLOYMENT_FILE=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$ESOL_DEPLOYMENT_FILE" ] && [ -f "$ESOL_DEPLOYMENT_FILE" ]; then
            ESOL_CONTRACT_ID=$(grep -o '"contract_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$ESOL_DEPLOYMENT_FILE" 2>/dev/null | sed -E 's/.*"contract_id"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
            if [ -n "$ESOL_CONTRACT_ID" ] && [ "$ESOL_CONTRACT_ID" != "null" ]; then
                break
            fi
        fi
    fi
done

# Verify eSOL contract ID was extracted
if [ -z "$ESOL_CONTRACT_ID" ] || [ "$ESOL_CONTRACT_ID" = "null" ]; then
    echo ""
    echo "ERROR: Could not extract eSOL contract_id from deployment file"
    echo "Current directory: $(pwd)"
    echo "Looking in: contract-deployments/"
    if [ -d "contract-deployments" ]; then
        echo "Files in contract-deployments/:"
        ls -la contract-deployments/ 2>/dev/null || echo "  (directory exists but listing failed)"
    else
        echo "ERROR: contract-deployments/ directory does not exist!"
    fi
    echo "eSOL contract deployment may have failed. Check the deployment output above."
    exit 1
fi

echo "eSOL contract ID extracted: $ESOL_CONTRACT_ID (from $ESOL_DEPLOYMENT_FILE)"

# Extract STAR contract_id from deployment file
STAR_CONTRACT_ID=""
STAR_DEPLOYMENT_FILE=""

# Try multiple possible filename patterns
POSSIBLE_STAR_PATTERNS=(
    "star-token-contract-*.json"
    "*star*.json"
)

for pattern in "${POSSIBLE_STAR_PATTERNS[@]}"; do
    if command -v jq &> /dev/null; then
        STAR_DEPLOYMENT_FILE=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$STAR_DEPLOYMENT_FILE" ] && [ -f "$STAR_DEPLOYMENT_FILE" ]; then
            STAR_CONTRACT_ID=$(jq -r '.contract_id // empty' "$STAR_DEPLOYMENT_FILE" 2>/dev/null)
            if [ -n "$STAR_CONTRACT_ID" ] && [ "$STAR_CONTRACT_ID" != "null" ] && [ "$STAR_CONTRACT_ID" != "empty" ]; then
                break
            fi
        fi
    else
        # Fallback: try to extract using grep/sed
        STAR_DEPLOYMENT_FILE=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$STAR_DEPLOYMENT_FILE" ] && [ -f "$STAR_DEPLOYMENT_FILE" ]; then
            STAR_CONTRACT_ID=$(grep -o '"contract_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$STAR_DEPLOYMENT_FILE" 2>/dev/null | sed -E 's/.*"contract_id"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
            if [ -n "$STAR_CONTRACT_ID" ] && [ "$STAR_CONTRACT_ID" != "null" ]; then
                break
            fi
        fi
    fi
done

# Verify STAR contract ID was extracted
if [ -z "$STAR_CONTRACT_ID" ] || [ "$STAR_CONTRACT_ID" = "null" ]; then
    echo ""
    echo "ERROR: Could not extract STAR contract_id from deployment file"
    echo "Current directory: $(pwd)"
    echo "Looking in: contract-deployments/"
    if [ -d "contract-deployments" ]; then
        echo "Files in contract-deployments/:"
        ls -la contract-deployments/ 2>/dev/null || echo "  (directory exists but listing failed)"
    else
        echo "ERROR: contract-deployments/ directory does not exist!"
    fi
    echo "STAR contract deployment may have failed. Check the deployment output above."
    exit 1
fi

echo "STAR contract ID extracted: $STAR_CONTRACT_ID (from $STAR_DEPLOYMENT_FILE)"

# Extract BasicReceiver contract_id from deployment file (contract ID changes when bytecode changes)
# Note: BASIC_RECEIVER_DEPLOYMENT_FILE should already be set from the verification step above
BASIC_RECEIVER_CONTRACT_ID=""

# Extract contract_id from the deployment file we already found
if [ -n "$BASIC_RECEIVER_DEPLOYMENT_FILE" ] && [ -f "$BASIC_RECEIVER_DEPLOYMENT_FILE" ]; then
    if command -v jq &> /dev/null; then
        BASIC_RECEIVER_CONTRACT_ID=$(jq -r '.contract_id // empty' "$BASIC_RECEIVER_DEPLOYMENT_FILE" 2>/dev/null)
    else
        # Fallback: try to extract using grep/sed
        BASIC_RECEIVER_CONTRACT_ID=$(grep -o '"contract_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$BASIC_RECEIVER_DEPLOYMENT_FILE" 2>/dev/null | sed -E 's/.*"contract_id"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
    fi
fi

# Verify BasicReceiver contract ID was extracted
if [ -z "$BASIC_RECEIVER_CONTRACT_ID" ] || [ "$BASIC_RECEIVER_CONTRACT_ID" = "null" ]; then
    echo ""
    echo "ERROR: Could not extract BasicReceiver contract_id from deployment file"
    echo "Current directory: $(pwd)"
    echo "Looking in: contract-deployments/"
    if [ -d "contract-deployments" ]; then
        echo "Files in contract-deployments/:"
        ls -la contract-deployments/ 2>/dev/null || echo "  (directory exists but listing failed)"
    else
        echo "ERROR: contract-deployments/ directory does not exist!"
    fi
    if [ -z "$BASIC_RECEIVER_DEPLOYMENT_FILE" ]; then
        echo "No BasicReceiver deployment file found matching any pattern:"
        for pattern in "${POSSIBLE_PATTERNS[@]}"; do
            echo "  - $pattern"
        done
    else
        echo "Deployment file found: $BASIC_RECEIVER_DEPLOYMENT_FILE"
        echo "But contract_id field is missing or invalid"
        echo "File contents:"
        cat "$BASIC_RECEIVER_DEPLOYMENT_FILE" 2>/dev/null | head -20
    fi
    echo "BasicReceiver contract deployment may have failed. Check the deployment output above."
    exit 1
fi

echo "BasicReceiver contract ID extracted: $BASIC_RECEIVER_CONTRACT_ID (from $BASIC_RECEIVER_DEPLOYMENT_FILE)"

# Extract AMM contract_id from deployment file
AMM_CONTRACT_ID=""
AMM_DEPLOYMENT_FILE=""

# Try multiple possible filename patterns
POSSIBLE_AMM_PATTERNS=(
    "amm-contract-*.json"
    "*amm*.json"
)

# First, try pattern matching
for pattern in "${POSSIBLE_AMM_PATTERNS[@]}"; do
if command -v jq &> /dev/null; then
        AMM_DEPLOYMENT_FILE=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$AMM_DEPLOYMENT_FILE" ] && [ -f "$AMM_DEPLOYMENT_FILE" ]; then
            AMM_CONTRACT_ID=$(jq -r '.contract_id // empty' "$AMM_DEPLOYMENT_FILE" 2>/dev/null)
            if [ -n "$AMM_CONTRACT_ID" ] && [ "$AMM_CONTRACT_ID" != "null" ] && [ "$AMM_CONTRACT_ID" != "empty" ]; then
                break
            fi
    fi
else
    # Fallback: try to extract using grep/sed
        AMM_DEPLOYMENT_FILE=$(ls -t contract-deployments/$pattern 2>/dev/null | head -1)
        if [ -n "$AMM_DEPLOYMENT_FILE" ] && [ -f "$AMM_DEPLOYMENT_FILE" ]; then
            AMM_CONTRACT_ID=$(grep -o '"contract_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$AMM_DEPLOYMENT_FILE" 2>/dev/null | sed -E 's/.*"contract_id"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
            if [ -n "$AMM_CONTRACT_ID" ] && [ "$AMM_CONTRACT_ID" != "null" ]; then
                break
            fi
        fi
    fi
done

# If still not found, try searching all JSON files by checking which one was created most recently after AMM deployment
if [ -z "$AMM_CONTRACT_ID" ] || [ "$AMM_CONTRACT_ID" = "null" ]; then
    # Find the most recently created JSON file that could be AMM
    # AMM is deployed last, so it should be the most recent file
    ALL_DEPLOYMENT_FILES=$(ls -t contract-deployments/*.json 2>/dev/null)
    if [ -n "$ALL_DEPLOYMENT_FILES" ]; then
        # Try the most recent file first (AMM is deployed last)
        for file in $ALL_DEPLOYMENT_FILES; do
            filename=$(basename "$file")
            # Skip known contract files that aren't AMM
            if [[ "$filename" == *"amm"* ]] || ([[ "$filename" != *"esol-token"* ]] && [[ "$filename" != *"star-token"* ]] && [[ "$filename" != *"basic"* ]] && [[ "$filename" != *"receiver"* ]]); then
                if command -v jq &> /dev/null; then
                    AMM_CONTRACT_ID=$(jq -r '.contract_id // empty' "$file" 2>/dev/null)
                    if [ -n "$AMM_CONTRACT_ID" ] && [ "$AMM_CONTRACT_ID" != "null" ] && [ "$AMM_CONTRACT_ID" != "empty" ]; then
                        AMM_DEPLOYMENT_FILE="$file"
                        break
                    fi
                else
                    AMM_CONTRACT_ID=$(grep -o '"contract_id"[[:space:]]*:[[:space:]]*"[^"]*"' "$file" 2>/dev/null | sed -E 's/.*"contract_id"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    if [ -n "$AMM_CONTRACT_ID" ] && [ "$AMM_CONTRACT_ID" != "null" ]; then
                        AMM_DEPLOYMENT_FILE="$file"
                        break
                    fi
                fi
            fi
        done
    fi
fi

# Verify AMM contract ID was extracted
if [ -z "$AMM_CONTRACT_ID" ] || [ "$AMM_CONTRACT_ID" = "null" ]; then
    echo ""
    echo "ERROR: Could not extract AMM contract_id from deployment file"
    echo "Current directory: $(pwd)"
    echo "Looking in: contract-deployments/"
    if [ -d "contract-deployments" ]; then
        echo "Files in contract-deployments/:"
        ls -la contract-deployments/ 2>/dev/null || echo "  (directory exists but listing failed)"
    else
        echo "ERROR: contract-deployments/ directory does not exist!"
    fi
    if [ -z "$AMM_DEPLOYMENT_FILE" ]; then
        echo "No AMM deployment file found matching any pattern:"
        for pattern in "${POSSIBLE_AMM_PATTERNS[@]}"; do
            echo "  - $pattern"
        done
    else
        echo "Deployment file found: $AMM_DEPLOYMENT_FILE"
        echo "But contract_id field is missing or invalid"
        echo "File contents:"
        cat "$AMM_DEPLOYMENT_FILE" 2>/dev/null | head -20
    fi
    echo "AMM contract deployment may have failed. Check the deployment output above."
    exit 1
fi

echo "AMM contract ID extracted: $AMM_CONTRACT_ID (from $AMM_DEPLOYMENT_FILE)"

# Send eSOL tokens to BasicReceiver contract
echo "Using eSOL contract_id: $ESOL_CONTRACT_ID"
echo "Using BasicReceiver contract_id: $BASIC_RECEIVER_CONTRACT_ID"

# Try sending eSOL tokens to BasicReceiver contract
echo ""
echo "Sending eSOL tokens to BasicReceiver contract..."
echo "Sending to BasicReceiver: $BASIC_RECEIVER_CONTRACT_ID"
echo "Amount: 666777888"
SEND_OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$ESOL_CONTRACT_ID" send "{\"contract\": \"$BASIC_RECEIVER_CONTRACT_ID\", \"amount\": \"666777888\", \"msg\": \"e30a\"}" 2>&1)
SEND_EXIT_CODE=$?
echo "$SEND_OUTPUT"
write_tx_response "esol_send_to_basic_receiver" "$SEND_OUTPUT"
if [ $SEND_EXIT_CODE -eq 0 ]; then
    echo "Successfully sent eSOL tokens to BasicReceiver contract"
else
    echo "Warning: Failed to send eSOL tokens to BasicReceiver contract"
fi

# Wait a moment for the transaction to be processed and committed
# This ensures nonce updates propagate before next transaction
sleep 3

# Try transferring eSOL to account (not contract) - independent of send result
echo ""
echo "Transferring eSOL tokens to account..."
echo "Recipient: 0x23b1f0b6199479b5d04fb54e21df14d51530b7b1"
echo "Amount: 888888888"
TRANSFER_OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$ESOL_CONTRACT_ID" transfer "{\"recipient\": \"0x23b1f0b6199479b5d04fb54e21df14d51530b7b1\", \"amount\": \"888888888\"}" 2>&1)
TRANSFER_EXIT_CODE=$?
echo "$TRANSFER_OUTPUT"
write_tx_response "esol_transfer" "$TRANSFER_OUTPUT"
if [ $TRANSFER_EXIT_CODE -eq 0 ]; then
    echo "Successfully transferred eSOL tokens to account"
else
    echo "Warning: Failed to transfer eSOL tokens to account"
fi

# Approve AMM for eSOL and STAR tokens, then add liquidity
# AMM_CONTRACT_ID is already verified above, so we can proceed
if [ -n "$AMM_CONTRACT_ID" ]; then
    echo ""
    echo "Approving AMM contract to spend tokens..."
    echo "Using eSOL contract_id: $ESOL_CONTRACT_ID"
    echo "Using STAR contract_id: $STAR_CONTRACT_ID"
    echo "Using AMM contract_id: $AMM_CONTRACT_ID"
    
    # Wallet2 address (from wallets.json) - matches DEV_WALLET
    WALLET1_ADDRESS="0x23b1f0b6199479b5d04fb54e21df14d51530b7b1"
    # Set large allowance (10^19) so we don't need to request approval repeatedly
    APPROVAL_AMOUNT="10000000000000000000"
    ESOL_APPROVED=false
    STAR_APPROVED=false
    
    # Approve AMM for eSOL tokens
    echo ""
    echo "Approving AMM contract to spend eSOL tokens..."
    echo "Amount: $APPROVAL_AMOUNT"
    MAX_RETRIES=3
    RETRY_COUNT=0
    ESOL_APPROVAL_SUCCESS=false
    
    while [ $RETRY_COUNT -lt $MAX_RETRIES ] && [ "$ESOL_APPROVAL_SUCCESS" = false ]; do
        # Get current account nonce before sending transaction
        echo "Getting current account nonce..."
        ACCOUNT_OUTPUT=$(cargo run -- get-account "$WALLET1_ADDRESS" 2>&1)
        
        # Try multiple patterns to extract nonce
        CURRENT_NONCE=$(echo "$ACCOUNT_OUTPUT" | grep -oE "nonce:[[:space:]]+[0-9]+" | grep -oE "[0-9]+" | head -1)
        
        # Fallback: try pattern with colon and space
        if [ -z "$CURRENT_NONCE" ]; then
            CURRENT_NONCE=$(echo "$ACCOUNT_OUTPUT" | grep -oE "nonce:[[:space:]]*[0-9]+" | grep -oE "[0-9]+" | head -1)
        fi
        
        # Fallback: try pattern with just "nonce" followed by number
        if [ -z "$CURRENT_NONCE" ]; then
            CURRENT_NONCE=$(echo "$ACCOUNT_OUTPUT" | grep -oE "\"nonce\"[[:space:]]*:[[:space:]]*[0-9]+" | grep -oE "[0-9]+" | head -1)
        fi
        
        if [ -z "$CURRENT_NONCE" ]; then
            echo "Warning: Could not extract current nonce from account output, assuming 0"
            echo "Account output (first 20 lines):"
            echo "$ACCOUNT_OUTPUT" | head -20
            CURRENT_NONCE=0
        else
            echo "Current account nonce: $CURRENT_NONCE"
        fi
        
        # Calculate expected nonce after transaction (current + 1)
        EXPECTED_NONCE=$((CURRENT_NONCE + 1))
        echo "Expected nonce after transaction: $EXPECTED_NONCE"
        
        OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$ESOL_CONTRACT_ID" increase_allowance "{\"spender\": \"$AMM_CONTRACT_ID\", \"amount\": \"$APPROVAL_AMOUNT\", \"expires\": null}" 2>&1)
    EXIT_CODE=$?
    echo "$OUTPUT"
    write_tx_response "esol_increase_allowance" "$OUTPUT"
        
        # Check if transaction succeeded (look for success message, not error messages)
        if [ $EXIT_CODE -eq 0 ] && echo "$OUTPUT" | grep -qi "Contract execution transaction was sent successfully"; then
            ESOL_APPROVAL_SUCCESS=true
            echo "eSOL approval transaction sent successfully"
            
            # Wait for account nonce to update (poll until it reaches expected value)
            if wait_for_nonce_update "$WALLET1_ADDRESS" "$EXPECTED_NONCE" 30; then
                echo "Account nonce updated successfully to $EXPECTED_NONCE"
            else
                echo "Warning: Account nonce did not update within timeout, but continuing..."
            fi
        else
            RETRY_COUNT=$((RETRY_COUNT + 1))
            if [ $RETRY_COUNT -lt $MAX_RETRIES ]; then
                echo "Warning: eSOL approval may have failed. Waiting before retry ($RETRY_COUNT/$MAX_RETRIES)..."
                sleep 5
    else
                echo "ERROR: eSOL approval failed after $MAX_RETRIES attempts"
            fi
        fi
    done
    
    # Verify the eSOL allowance was set
    echo ""
    echo "Verifying eSOL allowance..."
    ALLOWANCE_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" allowance "{\"owner\": \"$WALLET1_ADDRESS\", \"spender\": \"$AMM_CONTRACT_ID\"}" 2>&1)
    echo "$ALLOWANCE_OUTPUT"
    
    # Extract allowance value directly from output (handles multi-line JSON with log prefixes)
    # Method 1: Try to extract using jq (jq can parse JSON even with extra text)
    if command -v jq &> /dev/null; then
        ALLOWANCE_VALUE=$(echo "$ALLOWANCE_OUTPUT" | jq -r '.allowance // empty' 2>/dev/null)
    fi
    
    # Method 2: If jq didn't work, extract the value directly using grep/sed
    if [ -z "$ALLOWANCE_VALUE" ]; then
        # Find the line with "allowance" and extract the value
        ALLOWANCE_VALUE=$(echo "$ALLOWANCE_OUTPUT" | grep -o '"allowance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"allowance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
    fi
    
    # Method 3: Fallback - try to extract from any line containing allowance
    if [ -z "$ALLOWANCE_VALUE" ]; then
        ALLOWANCE_VALUE=$(echo "$ALLOWANCE_OUTPUT" | grep -i allowance | grep -o '"[0-9]*"' | tr -d '"' | head -1)
    fi
    
    # Check if we found an allowance value
    if [ -n "$ALLOWANCE_VALUE" ] && [ "$ALLOWANCE_VALUE" != "0" ] && [ "$ALLOWANCE_VALUE" != "null" ] && [ "$ALLOWANCE_VALUE" != "empty" ]; then
        echo "SUCCESS: eSOL allowance verified: $ALLOWANCE_VALUE"
        ESOL_APPROVED=true
    else
        echo "ERROR: eSOL allowance verification failed - allowance is 0 or not set"
        echo "Extracted allowance value: '$ALLOWANCE_VALUE'"
        echo "Full output: $ALLOWANCE_OUTPUT"
    fi
    
    # Approve AMM for STAR tokens
    echo ""
    echo "Approving AMM contract to spend STAR tokens..."
    echo "Amount: $APPROVAL_AMOUNT"
    MAX_RETRIES=3
    RETRY_COUNT=0
    STAR_APPROVAL_SUCCESS=false
    
    while [ $RETRY_COUNT -lt $MAX_RETRIES ] && [ "$STAR_APPROVAL_SUCCESS" = false ]; do
        # Get current account nonce before sending transaction
        echo "Getting current account nonce..."
        ACCOUNT_OUTPUT=$(cargo run -- get-account "$WALLET1_ADDRESS" 2>&1)
        
        # Try multiple patterns to extract nonce
        CURRENT_NONCE=$(echo "$ACCOUNT_OUTPUT" | grep -oE "nonce:[[:space:]]+[0-9]+" | grep -oE "[0-9]+" | head -1)
        
        # Fallback: try pattern with colon and space
        if [ -z "$CURRENT_NONCE" ]; then
            CURRENT_NONCE=$(echo "$ACCOUNT_OUTPUT" | grep -oE "nonce:[[:space:]]*[0-9]+" | grep -oE "[0-9]+" | head -1)
        fi
        
        # Fallback: try pattern with just "nonce" followed by number
        if [ -z "$CURRENT_NONCE" ]; then
            CURRENT_NONCE=$(echo "$ACCOUNT_OUTPUT" | grep -oE "\"nonce\"[[:space:]]*:[[:space:]]*[0-9]+" | grep -oE "[0-9]+" | head -1)
        fi
        
        if [ -z "$CURRENT_NONCE" ]; then
            echo "Warning: Could not extract current nonce from account output, assuming 0"
            echo "Account output (first 20 lines):"
            echo "$ACCOUNT_OUTPUT" | head -20
            CURRENT_NONCE=0
        else
            echo "Current account nonce: $CURRENT_NONCE"
        fi
        
        # Calculate expected nonce after transaction (current + 1)
        EXPECTED_NONCE=$((CURRENT_NONCE + 1))
        echo "Expected nonce after transaction: $EXPECTED_NONCE"
        
        OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$STAR_CONTRACT_ID" increase_allowance "{\"spender\": \"$AMM_CONTRACT_ID\", \"amount\": \"$APPROVAL_AMOUNT\", \"expires\": null}" 2>&1)
    EXIT_CODE=$?
    echo "$OUTPUT"
    write_tx_response "star_increase_allowance" "$OUTPUT"
        
        # Check if transaction succeeded (look for success message, not error messages)
        if [ $EXIT_CODE -eq 0 ] && echo "$OUTPUT" | grep -qi "Contract execution transaction was sent successfully"; then
            STAR_APPROVAL_SUCCESS=true
            echo "STAR approval transaction sent successfully"
            
            # Wait for account nonce to update (poll until it reaches expected value)
            if wait_for_nonce_update "$WALLET1_ADDRESS" "$EXPECTED_NONCE" 30; then
                echo "Account nonce updated successfully to $EXPECTED_NONCE"
            else
                echo "Warning: Account nonce did not update within timeout, but continuing..."
            fi
        else
            RETRY_COUNT=$((RETRY_COUNT + 1))
            if [ $RETRY_COUNT -lt $MAX_RETRIES ]; then
                echo "Warning: STAR approval may have failed. Waiting before retry ($RETRY_COUNT/$MAX_RETRIES)..."
                sleep 5
    else
                echo "ERROR: STAR approval failed after $MAX_RETRIES attempts"
            fi
        fi
    done
    
    # Verify the STAR allowance was set
    echo ""
    echo "Verifying STAR allowance..."
    ALLOWANCE_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" allowance "{\"owner\": \"$WALLET1_ADDRESS\", \"spender\": \"$AMM_CONTRACT_ID\"}" 2>&1)
    echo "$ALLOWANCE_OUTPUT"
    
    # Extract allowance value directly from output (handles multi-line JSON with log prefixes)
    # Method 1: Try to extract using jq (jq can parse JSON even with extra text)
    if command -v jq &> /dev/null; then
        ALLOWANCE_VALUE=$(echo "$ALLOWANCE_OUTPUT" | jq -r '.allowance // empty' 2>/dev/null)
    fi
    
    # Method 2: If jq didn't work, extract the value directly using grep/sed
    if [ -z "$ALLOWANCE_VALUE" ]; then
        # Find the line with "allowance" and extract the value
        ALLOWANCE_VALUE=$(echo "$ALLOWANCE_OUTPUT" | grep -o '"allowance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"allowance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
    fi
    
    # Method 3: Fallback - try to extract from any line containing allowance
    if [ -z "$ALLOWANCE_VALUE" ]; then
        ALLOWANCE_VALUE=$(echo "$ALLOWANCE_OUTPUT" | grep -i allowance | grep -o '"[0-9]*"' | tr -d '"' | head -1)
    fi
    
    # Check if we found an allowance value
    if [ -n "$ALLOWANCE_VALUE" ] && [ "$ALLOWANCE_VALUE" != "0" ] && [ "$ALLOWANCE_VALUE" != "null" ] && [ "$ALLOWANCE_VALUE" != "empty" ]; then
        echo "SUCCESS: STAR allowance verified: $ALLOWANCE_VALUE"
        STAR_APPROVED=true
    else
        echo "ERROR: STAR allowance verification failed - allowance is 0 or not set"
        echo "Extracted allowance value: '$ALLOWANCE_VALUE'"
        echo "Full output: $ALLOWANCE_OUTPUT"
    fi
    
    # Only proceed with add_liquidity if both approvals succeeded
    if [ "$ESOL_APPROVED" = true ] && [ "$STAR_APPROVED" = true ]; then
        echo ""
        echo "Both allowances verified successfully. Waiting for final confirmation before add_liquidity..."
        sleep 3
        
        # Final verification of both allowances right before add_liquidity
        echo "Performing final allowance verification..."
        
        # Check eSOL allowance using robust extraction method (same as earlier verification)
        echo "Checking final eSOL allowance..."
        ESOL_FINAL_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" allowance "{\"owner\": \"$WALLET1_ADDRESS\", \"spender\": \"$AMM_CONTRACT_ID\"}" 2>&1)
        echo "$ESOL_FINAL_OUTPUT"
        
        # Method 1: Try to extract using jq (jq can parse JSON even with extra text)
        ESOL_FINAL_CHECK=""
        if command -v jq &> /dev/null; then
            ESOL_FINAL_CHECK=$(echo "$ESOL_FINAL_OUTPUT" | jq -r '.allowance // empty' 2>/dev/null)
        fi
        
        # Method 2: If jq didn't work, extract the value directly using grep/sed
        if [ -z "$ESOL_FINAL_CHECK" ]; then
            ESOL_FINAL_CHECK=$(echo "$ESOL_FINAL_OUTPUT" | grep -o '"allowance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"allowance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
        fi
        
        # Method 3: Fallback - try to extract from any line containing allowance
        if [ -z "$ESOL_FINAL_CHECK" ]; then
            ESOL_FINAL_CHECK=$(echo "$ESOL_FINAL_OUTPUT" | grep -i allowance | grep -o '"[0-9]*"' | tr -d '"' | head -1)
        fi
        
        # Check if we found an allowance value
        if [ -z "$ESOL_FINAL_CHECK" ] || [ "$ESOL_FINAL_CHECK" = "0" ] || [ "$ESOL_FINAL_CHECK" = "null" ] || [ "$ESOL_FINAL_CHECK" = "empty" ]; then
            echo "ERROR: Final eSOL allowance check failed: $ESOL_FINAL_CHECK"
            echo "Extracted allowance value: '$ESOL_FINAL_CHECK'"
            echo "Full output: $ESOL_FINAL_OUTPUT"
            ESOL_APPROVED=false
        else
            echo "SUCCESS: Final eSOL allowance verified: $ESOL_FINAL_CHECK"
        fi
        
        # Check STAR allowance using robust extraction method (same as earlier verification)
        echo ""
        echo "Checking final STAR allowance..."
        STAR_FINAL_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" allowance "{\"owner\": \"$WALLET1_ADDRESS\", \"spender\": \"$AMM_CONTRACT_ID\"}" 2>&1)
        echo "$STAR_FINAL_OUTPUT"
        
        # Method 1: Try to extract using jq (jq can parse JSON even with extra text)
        STAR_FINAL_CHECK=""
        if command -v jq &> /dev/null; then
            STAR_FINAL_CHECK=$(echo "$STAR_FINAL_OUTPUT" | jq -r '.allowance // empty' 2>/dev/null)
        fi
        
        # Method 2: If jq didn't work, extract the value directly using grep/sed
        if [ -z "$STAR_FINAL_CHECK" ]; then
            STAR_FINAL_CHECK=$(echo "$STAR_FINAL_OUTPUT" | grep -o '"allowance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"allowance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
        fi
        
        # Method 3: Fallback - try to extract from any line containing allowance
        if [ -z "$STAR_FINAL_CHECK" ]; then
            STAR_FINAL_CHECK=$(echo "$STAR_FINAL_OUTPUT" | grep -i allowance | grep -o '"[0-9]*"' | tr -d '"' | head -1)
        fi
        
        # Check if we found an allowance value
        if [ -z "$STAR_FINAL_CHECK" ] || [ "$STAR_FINAL_CHECK" = "0" ] || [ "$STAR_FINAL_CHECK" = "null" ] || [ "$STAR_FINAL_CHECK" = "empty" ]; then
            echo "ERROR: Final STAR allowance check failed: $STAR_FINAL_CHECK"
            echo "Extracted allowance value: '$STAR_FINAL_CHECK'"
            echo "Full output: $STAR_FINAL_OUTPUT"
            STAR_APPROVED=false
        else
            echo "SUCCESS: Final STAR allowance verified: $STAR_FINAL_CHECK"
        fi
        
        if [ "$ESOL_APPROVED" = true ] && [ "$STAR_APPROVED" = true ]; then
            echo "Final verification passed. eSOL allowance: $ESOL_FINAL_CHECK, STAR allowance: $STAR_FINAL_CHECK"
    echo ""
    echo "Adding liquidity to AMM pool..."
            # Use a reasonable liquidity amount (not the allowance amount)
            # The allowance is set to max, but we only add a reasonable amount of liquidity
            LIQUIDITY_AMOUNT="500000000"
            echo "Token1 (eSOL) amount: $LIQUIDITY_AMOUNT"
            echo "Token2 (STAR) amount: $LIQUIDITY_AMOUNT"
    echo "AMM Contract ID: $AMM_CONTRACT_ID"
    
            MAX_RETRIES=3
            RETRY_DELAY=3
            ADD_LIQUIDITY_SUCCESS=false
            
            for RETRY_ATTEMPT in $(seq 0 $MAX_RETRIES); do
                if [ $RETRY_ATTEMPT -gt 0 ]; then
                    echo ""
                    echo "Retry attempt $RETRY_ATTEMPT/$MAX_RETRIES: Waiting $RETRY_DELAY seconds before retry..."
                    sleep $RETRY_DELAY
                fi
    
                OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$AMM_CONTRACT_ID" add_liquidity "{\"token1_amount\": \"$LIQUIDITY_AMOUNT\", \"min_liquidity\": \"$LIQUIDITY_AMOUNT\", \"max_token2\": \"$LIQUIDITY_AMOUNT\", \"expiration\": null}" 2>&1)
    EXIT_CODE=$?
    echo "$OUTPUT"
    write_tx_response "amm_add_liquidity" "$OUTPUT"
    
                # Check if it's a nonce error
                if echo "$OUTPUT" | grep -qi "code 3\|Nonce is not sequential\|First nonce should be 0"; then
                    if [ $RETRY_ATTEMPT -lt $MAX_RETRIES ]; then
                        echo "WARNING: Nonce error detected. Will retry after waiting..."
                        continue
                    else
                        echo "ERROR: Nonce error after $MAX_RETRIES retries. Giving up."
                        break
                    fi
                fi
                
                # Check for success
                if [ $EXIT_CODE -eq 0 ] && echo "$OUTPUT" | grep -qi "successfully\|sent successfully"; then
                    ADD_LIQUIDITY_SUCCESS=true
                    break
                fi
                
                # Check for other errors
                if [ $EXIT_CODE -ne 0 ] || echo "$OUTPUT" | grep -qi "FAILED\|Error\|error\|rejected\|invalid"; then
                    if [ $RETRY_ATTEMPT -lt $MAX_RETRIES ]; then
                        echo "WARNING: Transaction failed. Will retry after waiting..."
                        continue
                    else
                        echo "ERROR: Add liquidity transaction failed after $MAX_RETRIES retries."
                        break
                    fi
                fi
            done
            
            if [ "$ADD_LIQUIDITY_SUCCESS" = true ]; then
        echo "Transaction appears to have been sent successfully"
        # Wait for transaction to be processed
        sleep 3
        
        # Verify liquidity was added by querying the AMM
        echo ""
        echo "Verifying liquidity was added..."
        QUERY_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" info '{}' 2>&1)
        echo "$QUERY_OUTPUT"
        
        # Check if reserves are non-zero
        if echo "$QUERY_OUTPUT" | grep -q '"token1_reserve": "0"'; then
            echo "WARNING: token1_reserve is still 0 - liquidity may not have been added"
        else
            echo "SUCCESS: Liquidity appears to have been added (reserves are non-zero)"
                
                # Extract LP token address from AMM info
                echo ""
                echo "Extracting LP token address..."
                if command -v jq &> /dev/null; then
                    LP_TOKEN_ADDRESS=$(echo "$QUERY_OUTPUT" | jq -r '.lp_token_address // empty' 2>/dev/null)
                fi
                if [ -z "$LP_TOKEN_ADDRESS" ]; then
                    LP_TOKEN_ADDRESS=$(echo "$QUERY_OUTPUT" | grep -o '"lp_token_address"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"lp_token_address"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                
                # Ensure LP token address has 0x prefix
                if [ -n "$LP_TOKEN_ADDRESS" ] && [ "$LP_TOKEN_ADDRESS" != "null" ] && [ "$LP_TOKEN_ADDRESS" != "empty" ]; then
                    if [[ ! "$LP_TOKEN_ADDRESS" =~ ^0x ]]; then
                        LP_TOKEN_ADDRESS="0x$LP_TOKEN_ADDRESS"
                    fi
                    echo ">>> LP Token Address: $LP_TOKEN_ADDRESS"
                    echo "LP_TOKEN_ADDRESS: $LP_TOKEN_ADDRESS"
                    
                    # Query LP token balance for wallet2
                    echo ""
                    echo "Querying LP token balance for wallet2..."
                    echo "LP_TOKEN_ADDRESS: $LP_TOKEN_ADDRESS"
                    LP_BALANCE_OUTPUT=$(cargo run -- contract-query2 "$LP_TOKEN_ADDRESS" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                    echo "$LP_BALANCE_OUTPUT"
                    
                    # Extract LP token balance
                    if command -v jq &> /dev/null; then
                        LP_BALANCE=$(echo "$LP_BALANCE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                    fi
                    if [ -z "$LP_BALANCE" ]; then
                        LP_BALANCE=$(echo "$LP_BALANCE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -n "$LP_BALANCE" ] && [ "$LP_BALANCE" != "null" ] && [ "$LP_BALANCE" != "empty" ]; then
                        echo ">>> LP Token Balance: $LP_BALANCE"
                    else
                        echo ">>> LP Token Balance: Could not extract balance"
                    fi
                else
                    echo ">>> WARNING: Could not extract LP token address from AMM info"
                fi
                
                # Query prices after liquidity is confirmed
                echo ""
                echo "=========================================="
                echo "Querying AMM prices..."
                echo "=========================================="
                
                # Query price for token1 (how much token2 you get for 1 token1)
                # Using 1000000 = 1 token (with 6 decimals)
                PRICE_QUERY_AMOUNT="1000000"
                echo ""
                echo "Querying eSOL price (how much STAR for $PRICE_QUERY_AMOUNT eSOL)..."
                TOKEN1_PRICE_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token1_for_token2_price "{\"token1_amount\": \"$PRICE_QUERY_AMOUNT\"}" 2>&1)
                echo "$TOKEN1_PRICE_OUTPUT"
                
                # Extract token2_amount from the response
                if command -v jq &> /dev/null; then
                    TOKEN2_AMOUNT=$(echo "$TOKEN1_PRICE_OUTPUT" | jq -r '.token2_amount // empty' 2>/dev/null)
                fi
                if [ -z "$TOKEN2_AMOUNT" ]; then
                    TOKEN2_AMOUNT=$(echo "$TOKEN1_PRICE_OUTPUT" | grep -o '"token2_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token2_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$TOKEN2_AMOUNT" ] && [ "$TOKEN2_AMOUNT" != "null" ] && [ "$TOKEN2_AMOUNT" != "empty" ]; then
                    echo ">>> eSOL Price: $PRICE_QUERY_AMOUNT eSOL = $TOKEN2_AMOUNT STAR"
                else
                    echo ">>> eSOL Price: Could not extract price from response"
                fi
                
                # Query price for token2 (how much token1 you get for 1 token2)
                echo ""
                echo "Querying STAR price (how much eSOL for $PRICE_QUERY_AMOUNT STAR)..."
                TOKEN2_PRICE_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token2_for_token1_price "{\"token2_amount\": \"$PRICE_QUERY_AMOUNT\"}" 2>&1)
                echo "$TOKEN2_PRICE_OUTPUT"
                
                # Extract token1_amount from the response
                if command -v jq &> /dev/null; then
                    TOKEN1_AMOUNT=$(echo "$TOKEN2_PRICE_OUTPUT" | jq -r '.token1_amount // empty' 2>/dev/null)
                fi
                if [ -z "$TOKEN1_AMOUNT" ]; then
                    TOKEN1_AMOUNT=$(echo "$TOKEN2_PRICE_OUTPUT" | grep -o '"token1_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token1_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$TOKEN1_AMOUNT" ] && [ "$TOKEN1_AMOUNT" != "null" ] && [ "$TOKEN1_AMOUNT" != "empty" ]; then
                    echo ">>> STAR Price: $PRICE_QUERY_AMOUNT STAR = $TOKEN1_AMOUNT eSOL"
                else
                    echo ">>> STAR Price: Could not extract price from response"
                fi
                
                echo ""
                echo "=========================================="
                
                # Execute swap: Buy 1000 STAR tokens with eSOL
                echo ""
                echo "=========================================="
                echo "Executing swap: Buy 1000 STAR with eSOL"
                echo "=========================================="
                
                # Query balances before swap
                echo ""
                echo "Querying balances BEFORE swap..."
                echo "Querying eSOL balance..."
                ESOL_BALANCE_BEFORE_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                echo "$ESOL_BALANCE_BEFORE_OUTPUT"
                
                # Extract eSOL balance
                if command -v jq &> /dev/null; then
                    ESOL_BALANCE_BEFORE=$(echo "$ESOL_BALANCE_BEFORE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                fi
                if [ -z "$ESOL_BALANCE_BEFORE" ]; then
                    ESOL_BALANCE_BEFORE=$(echo "$ESOL_BALANCE_BEFORE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$ESOL_BALANCE_BEFORE" ] && [ "$ESOL_BALANCE_BEFORE" != "null" ] && [ "$ESOL_BALANCE_BEFORE" != "empty" ]; then
                    echo ">>> eSOL Balance BEFORE: $ESOL_BALANCE_BEFORE"
                else
                    echo ">>> eSOL Balance BEFORE: Could not extract balance"
                    ESOL_BALANCE_BEFORE="0"
                fi
                
                echo ""
                echo "Querying STAR balance..."
                STAR_BALANCE_BEFORE_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                echo "$STAR_BALANCE_BEFORE_OUTPUT"
                
                # Extract STAR balance
                if command -v jq &> /dev/null; then
                    STAR_BALANCE_BEFORE=$(echo "$STAR_BALANCE_BEFORE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                fi
                if [ -z "$STAR_BALANCE_BEFORE" ]; then
                    STAR_BALANCE_BEFORE=$(echo "$STAR_BALANCE_BEFORE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$STAR_BALANCE_BEFORE" ] && [ "$STAR_BALANCE_BEFORE" != "null" ] && [ "$STAR_BALANCE_BEFORE" != "empty" ]; then
                    echo ">>> STAR Balance BEFORE: $STAR_BALANCE_BEFORE"
                else
                    echo ">>> STAR Balance BEFORE: Could not extract balance"
                    STAR_BALANCE_BEFORE="0"
                fi
                
                # Calculate how much eSOL we need for 1 STAR token (1000000 with 6 decimals, small amount for testing)
                # We'll query the price first to get an estimate, then use a slightly higher amount
                STAR_TARGET_AMOUNT="1000000"  # 1 STAR token (with 6 decimals) - small amount for testing
                echo ""
                echo "Querying price to estimate eSOL needed for $STAR_TARGET_AMOUNT STAR..."
                PRICE_ESTIMATE_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token2_for_token1_price "{\"token2_amount\": \"$STAR_TARGET_AMOUNT\"}" 2>&1)
                echo "$PRICE_ESTIMATE_OUTPUT"
                
                # Extract estimated eSOL amount needed
                if command -v jq &> /dev/null; then
                    ESOL_ESTIMATE=$(echo "$PRICE_ESTIMATE_OUTPUT" | jq -r '.token1_amount // empty' 2>/dev/null)
                fi
                if [ -z "$ESOL_ESTIMATE" ]; then
                    ESOL_ESTIMATE=$(echo "$PRICE_ESTIMATE_OUTPUT" | grep -o '"token1_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token1_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                
                # Use the estimate, or default to 1000000 if we can't get it
                if [ -n "$ESOL_ESTIMATE" ] && [ "$ESOL_ESTIMATE" != "null" ] && [ "$ESOL_ESTIMATE" != "empty" ]; then
                    # Add 5% buffer to ensure we get the minimum output
                    ESOL_INPUT_AMOUNT=$(echo "scale=0; $ESOL_ESTIMATE * 105 / 100" | bc 2>/dev/null || echo "$ESOL_ESTIMATE")
                    echo ">>> Estimated eSOL needed: $ESOL_ESTIMATE (using $ESOL_INPUT_AMOUNT with 5% buffer)"
                else
                    ESOL_INPUT_AMOUNT="1000000"  # Default to 1 eSOL if we can't get estimate
                    echo ">>> Could not get price estimate, using default: $ESOL_INPUT_AMOUNT eSOL"
                fi
                
                # Query the actual output amount we'll get for this input amount
                # This accounts for fees and slippage, so we can set a realistic min_output
                echo ""
                echo "Querying actual STAR output for $ESOL_INPUT_AMOUNT eSOL input..."
                ACTUAL_OUTPUT_QUERY=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token1_for_token2_price "{\"token1_amount\": \"$ESOL_INPUT_AMOUNT\"}" 2>&1)
                echo "$ACTUAL_OUTPUT_QUERY"
                
                # Extract actual output amount
                if command -v jq &> /dev/null; then
                    ACTUAL_STAR_OUTPUT=$(echo "$ACTUAL_OUTPUT_QUERY" | jq -r '.token2_amount // empty' 2>/dev/null)
                fi
                if [ -z "$ACTUAL_STAR_OUTPUT" ]; then
                    ACTUAL_STAR_OUTPUT=$(echo "$ACTUAL_OUTPUT_QUERY" | grep -o '"token2_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token2_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                
                # Set min_output to 95% of actual output to account for any small changes between query and execution
                if [ -n "$ACTUAL_STAR_OUTPUT" ] && [ "$ACTUAL_STAR_OUTPUT" != "null" ] && [ "$ACTUAL_STAR_OUTPUT" != "empty" ]; then
                    MIN_OUTPUT_AMOUNT=$(echo "scale=0; $ACTUAL_STAR_OUTPUT * 95 / 100" | bc 2>/dev/null || echo "$ACTUAL_STAR_OUTPUT")
                    echo ">>> Actual STAR output for $ESOL_INPUT_AMOUNT eSOL: $ACTUAL_STAR_OUTPUT"
                    echo ">>> Setting min_output to $MIN_OUTPUT_AMOUNT (95% of actual output for safety)"
                else
                    # Fallback: use 90% of target amount if we can't get actual output
                    MIN_OUTPUT_AMOUNT=$(echo "scale=0; $STAR_TARGET_AMOUNT * 90 / 100" | bc 2>/dev/null || echo "900000")
                    echo ">>> Could not get actual output, using fallback min_output: $MIN_OUTPUT_AMOUNT (90% of target)"
                fi
                
                # Check current eSOL allowance (may have been consumed during add_liquidity)
                echo ""
                echo "Checking current eSOL allowance for AMM contract..."
                CURRENT_ALLOWANCE_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" allowance "{\"owner\": \"$WALLET1_ADDRESS\", \"spender\": \"$AMM_CONTRACT_ID\"}" 2>&1)
                echo "$CURRENT_ALLOWANCE_OUTPUT"
                
                # Extract current allowance value
                if command -v jq &> /dev/null; then
                    CURRENT_ALLOWANCE=$(echo "$CURRENT_ALLOWANCE_OUTPUT" | jq -r '.allowance // empty' 2>/dev/null)
                fi
                if [ -z "$CURRENT_ALLOWANCE" ]; then
                    CURRENT_ALLOWANCE=$(echo "$CURRENT_ALLOWANCE_OUTPUT" | grep -o '"allowance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"allowance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -z "$CURRENT_ALLOWANCE" ] || [ "$CURRENT_ALLOWANCE" = "null" ] || [ "$CURRENT_ALLOWANCE" = "empty" ]; then
                    CURRENT_ALLOWANCE="0"
                fi
                
                echo ">>> Current eSOL allowance: $CURRENT_ALLOWANCE"
                
                # Only increase allowance if current allowance is insufficient
                # Add 5% buffer to the required amount
                REQUIRED_ALLOWANCE=$(echo "scale=0; $ESOL_INPUT_AMOUNT * 105 / 100" | bc 2>/dev/null || echo "$ESOL_INPUT_AMOUNT")
                
                if [ "$CURRENT_ALLOWANCE" -lt "$REQUIRED_ALLOWANCE" ] 2>/dev/null; then
                    echo ""
                    echo "Current allowance ($CURRENT_ALLOWANCE) is insufficient. Increasing allowance to $REQUIRED_ALLOWANCE..."
                    ALLOWANCE_TO_ADD=$(echo "scale=0; $REQUIRED_ALLOWANCE - $CURRENT_ALLOWANCE" | bc 2>/dev/null || echo "$REQUIRED_ALLOWANCE")
                    ALLOWANCE_OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$ESOL_CONTRACT_ID" increase_allowance "{\"spender\": \"$AMM_CONTRACT_ID\", \"amount\": \"$ALLOWANCE_TO_ADD\", \"expires\": null}" 2>&1)
                    write_tx_response "esol_increase_allowance_swap" "$ALLOWANCE_OUTPUT"
                    EXIT_CODE=$?
                    echo "$ALLOWANCE_OUTPUT"
                    
                    if [ $EXIT_CODE -eq 0 ] && echo "$ALLOWANCE_OUTPUT" | grep -qi "Contract execution transaction was sent successfully"; then
                        echo "eSOL allowance increased successfully"
                        sleep 3
                    else
                        echo "WARNING: Setting allowance may have failed, but continuing with swap attempt..."
                        sleep 2
                    fi
                else
                    echo ">>> Current allowance ($CURRENT_ALLOWANCE) is sufficient for swap (need $REQUIRED_ALLOWANCE)"
                fi
                
                # Check AMM's actual token balances before swap (for debugging)
                echo ""
                echo "Checking AMM contract token balances..."
                echo "Querying AMM's eSOL balance..."
                AMM_ESOL_BALANCE_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" balance "{\"address\": \"$AMM_CONTRACT_ID\"}" 2>&1)
                echo "$AMM_ESOL_BALANCE_OUTPUT"
                
                echo ""
                echo "Querying AMM's STAR balance..."
                AMM_STAR_BALANCE_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" balance "{\"address\": \"$AMM_CONTRACT_ID\"}" 2>&1)
                echo "$AMM_STAR_BALANCE_OUTPUT"
                
                # Query AMM reserves for comparison
                echo ""
                echo "Querying AMM reserves..."
                AMM_INFO_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" info '{}' 2>&1)
                echo "$AMM_INFO_OUTPUT"
                
                # Execute the swap with retry logic for nonce errors
                echo ""
                echo "Executing swap: Buying STAR with eSOL..."
                echo "Input: $ESOL_INPUT_AMOUNT eSOL (Token1)"
                echo "Target: ~$STAR_TARGET_AMOUNT STAR (Token2)"
                echo "Minimum output: $MIN_OUTPUT_AMOUNT STAR (Token2) - protects against slippage"
                
                MAX_RETRIES=3
                RETRY_DELAY=3
                SWAP_SUCCESS=false
                
                for RETRY_ATTEMPT in $(seq 0 $MAX_RETRIES); do
                    if [ $RETRY_ATTEMPT -gt 0 ]; then
                        echo ""
                        echo "Retry attempt $RETRY_ATTEMPT/$MAX_RETRIES: Waiting $RETRY_DELAY seconds before retry..."
                        sleep $RETRY_DELAY
                    fi
                    
                    SWAP_OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$AMM_CONTRACT_ID" swap "{\"input_token\": \"Token1\", \"input_amount\": \"$ESOL_INPUT_AMOUNT\", \"min_output\": \"$MIN_OUTPUT_AMOUNT\", \"expiration\": null}" 2>&1)
                    write_tx_response "amm_swap_esol_to_star" "$SWAP_OUTPUT"
                    EXIT_CODE=$?
                    echo "$SWAP_OUTPUT"
                    
                    # Check if it's a nonce error
                    if echo "$SWAP_OUTPUT" | grep -qi "code 3\|Nonce is not sequential\|First nonce should be 0"; then
                        if [ $RETRY_ATTEMPT -lt $MAX_RETRIES ]; then
                            echo "WARNING: Nonce error detected. Will retry after waiting..."
                            continue
                        else
                            echo "ERROR: Nonce error after $MAX_RETRIES retries. Giving up."
                            break
                        fi
                    fi
                    
                    # Check for success
                    if [ $EXIT_CODE -eq 0 ] && echo "$SWAP_OUTPUT" | grep -qi "successfully\|sent successfully"; then
                        SWAP_SUCCESS=true
                        break
                    fi
                    
                    # Check for other errors
                    if [ $EXIT_CODE -ne 0 ] || echo "$SWAP_OUTPUT" | grep -qi "FAILED\|Error\|error\|rejected\|invalid"; then
                        if [ $RETRY_ATTEMPT -lt $MAX_RETRIES ]; then
                            echo "WARNING: Transaction failed. Will retry after waiting..."
                            continue
                        else
                            echo "ERROR: Swap transaction failed after $MAX_RETRIES retries."
                            break
                        fi
                    fi
                done
                
                if [ "$SWAP_SUCCESS" = true ]; then
                    echo "Swap transaction sent successfully"
                    # Wait for transaction to be processed
                    sleep 5
                    
                    # Query balances after swap
                    echo ""
                    echo "=========================================="
                    echo "Querying balances AFTER swap..."
                    echo "=========================================="
                    echo ""
                    echo "Querying eSOL balance..."
                    ESOL_BALANCE_AFTER_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                    echo "$ESOL_BALANCE_AFTER_OUTPUT"
                    
                    # Extract eSOL balance
                    if command -v jq &> /dev/null; then
                        ESOL_BALANCE_AFTER=$(echo "$ESOL_BALANCE_AFTER_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                    fi
                    if [ -z "$ESOL_BALANCE_AFTER" ]; then
                        ESOL_BALANCE_AFTER=$(echo "$ESOL_BALANCE_AFTER_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -n "$ESOL_BALANCE_AFTER" ] && [ "$ESOL_BALANCE_AFTER" != "null" ] && [ "$ESOL_BALANCE_AFTER" != "empty" ]; then
                        echo ">>> eSOL Balance AFTER: $ESOL_BALANCE_AFTER"
                    else
                        echo ">>> eSOL Balance AFTER: Could not extract balance"
                        ESOL_BALANCE_AFTER="0"
                    fi
                    
                    echo ""
                    echo "Querying STAR balance..."
                    STAR_BALANCE_AFTER_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                    echo "$STAR_BALANCE_AFTER_OUTPUT"
                    
                    # Extract STAR balance
                    if command -v jq &> /dev/null; then
                        STAR_BALANCE_AFTER=$(echo "$STAR_BALANCE_AFTER_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                    fi
                    if [ -z "$STAR_BALANCE_AFTER" ]; then
                        STAR_BALANCE_AFTER=$(echo "$STAR_BALANCE_AFTER_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -n "$STAR_BALANCE_AFTER" ] && [ "$STAR_BALANCE_AFTER" != "null" ] && [ "$STAR_BALANCE_AFTER" != "empty" ]; then
                        echo ">>> STAR Balance AFTER: $STAR_BALANCE_AFTER"
                    else
                        echo ">>> STAR Balance AFTER: Could not extract balance"
                        STAR_BALANCE_AFTER="0"
                    fi
                    
                    # Calculate changes
                    echo ""
                    echo "Balance Changes:"
                    if [ "$ESOL_BALANCE_BEFORE" != "0" ] && [ "$ESOL_BALANCE_AFTER" != "0" ]; then
                        ESOL_CHANGE=$(echo "scale=0; $ESOL_BALANCE_AFTER - $ESOL_BALANCE_BEFORE" | bc 2>/dev/null || echo "N/A")
                        echo ">>> eSOL Change: $ESOL_CHANGE"
                    fi
                    if [ "$STAR_BALANCE_BEFORE" != "0" ] && [ "$STAR_BALANCE_AFTER" != "0" ]; then
                        STAR_CHANGE=$(echo "scale=0; $STAR_BALANCE_AFTER - $STAR_BALANCE_BEFORE" | bc 2>/dev/null || echo "N/A")
                        echo ">>> STAR Change: $STAR_CHANGE"
                    fi
                    
                    # Query prices after swap
                    echo ""
                    echo "=========================================="
                    echo "Querying prices AFTER swap..."
                    echo "=========================================="
                    echo ""
                    echo "Querying eSOL price (how much STAR for $PRICE_QUERY_AMOUNT eSOL)..."
                    TOKEN1_PRICE_AFTER_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token1_for_token2_price "{\"token1_amount\": \"$PRICE_QUERY_AMOUNT\"}" 2>&1)
                    echo "$TOKEN1_PRICE_AFTER_OUTPUT"
                    
                    # Extract token2_amount from the response
                    if command -v jq &> /dev/null; then
                        TOKEN2_AMOUNT_AFTER=$(echo "$TOKEN1_PRICE_AFTER_OUTPUT" | jq -r '.token2_amount // empty' 2>/dev/null)
                    fi
                    if [ -z "$TOKEN2_AMOUNT_AFTER" ]; then
                        TOKEN2_AMOUNT_AFTER=$(echo "$TOKEN1_PRICE_AFTER_OUTPUT" | grep -o '"token2_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token2_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -n "$TOKEN2_AMOUNT_AFTER" ] && [ "$TOKEN2_AMOUNT_AFTER" != "null" ] && [ "$TOKEN2_AMOUNT_AFTER" != "empty" ]; then
                        echo ">>> eSOL Price AFTER: $PRICE_QUERY_AMOUNT eSOL = $TOKEN2_AMOUNT_AFTER STAR"
                    else
                        echo ">>> eSOL Price AFTER: Could not extract price from response"
                    fi
                    
                    echo ""
                    echo "Querying STAR price (how much eSOL for $PRICE_QUERY_AMOUNT STAR)..."
                    TOKEN2_PRICE_AFTER_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token2_for_token1_price "{\"token2_amount\": \"$PRICE_QUERY_AMOUNT\"}" 2>&1)
                    echo "$TOKEN2_PRICE_AFTER_OUTPUT"
                    
                    # Extract token1_amount from the response
                    if command -v jq &> /dev/null; then
                        TOKEN1_AMOUNT_AFTER=$(echo "$TOKEN2_PRICE_AFTER_OUTPUT" | jq -r '.token1_amount // empty' 2>/dev/null)
                    fi
                    if [ -z "$TOKEN1_AMOUNT_AFTER" ]; then
                        TOKEN1_AMOUNT_AFTER=$(echo "$TOKEN2_PRICE_AFTER_OUTPUT" | grep -o '"token1_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token1_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -n "$TOKEN1_AMOUNT_AFTER" ] && [ "$TOKEN1_AMOUNT_AFTER" != "null" ] && [ "$TOKEN1_AMOUNT_AFTER" != "empty" ]; then
                        echo ">>> STAR Price AFTER: $PRICE_QUERY_AMOUNT STAR = $TOKEN1_AMOUNT_AFTER eSOL"
                    else
                        echo ">>> STAR Price AFTER: Could not extract price from response"
                    fi
                    
                    echo ""
                    echo "=========================================="
                else
                    echo "WARNING: Could not determine swap transaction status"
                fi
                
                # ==========================================
                # Reverse Swap: Buy eSOL with STAR
                # ==========================================
                echo ""
                echo "=========================================="
                echo "=========================================="
                echo "Executing reverse swap: Buy eSOL with STAR"
                echo "=========================================="
                echo "=========================================="
                
                # Use smaller amounts for reverse swap
                ESOL_TARGET_AMOUNT="500000"  # 0.5 eSOL (with 6 decimals) - smaller amount
                echo ""
                echo "Querying balances BEFORE reverse swap..."
                echo "Querying eSOL balance..."
                ESOL_BALANCE_BEFORE_REVERSE_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                echo "$ESOL_BALANCE_BEFORE_REVERSE_OUTPUT"
                
                # Extract eSOL balance
                if command -v jq &> /dev/null; then
                    ESOL_BALANCE_BEFORE_REVERSE=$(echo "$ESOL_BALANCE_BEFORE_REVERSE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                fi
                if [ -z "$ESOL_BALANCE_BEFORE_REVERSE" ]; then
                    ESOL_BALANCE_BEFORE_REVERSE=$(echo "$ESOL_BALANCE_BEFORE_REVERSE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$ESOL_BALANCE_BEFORE_REVERSE" ] && [ "$ESOL_BALANCE_BEFORE_REVERSE" != "null" ] && [ "$ESOL_BALANCE_BEFORE_REVERSE" != "empty" ]; then
                    echo ">>> eSOL Balance BEFORE reverse swap: $ESOL_BALANCE_BEFORE_REVERSE"
                else
                    echo ">>> eSOL Balance BEFORE reverse swap: Could not extract balance"
                    ESOL_BALANCE_BEFORE_REVERSE="0"
                fi
                
                echo ""
                echo "Querying STAR balance..."
                STAR_BALANCE_BEFORE_REVERSE_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                echo "$STAR_BALANCE_BEFORE_REVERSE_OUTPUT"
                
                # Extract STAR balance
                if command -v jq &> /dev/null; then
                    STAR_BALANCE_BEFORE_REVERSE=$(echo "$STAR_BALANCE_BEFORE_REVERSE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                fi
                if [ -z "$STAR_BALANCE_BEFORE_REVERSE" ]; then
                    STAR_BALANCE_BEFORE_REVERSE=$(echo "$STAR_BALANCE_BEFORE_REVERSE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$STAR_BALANCE_BEFORE_REVERSE" ] && [ "$STAR_BALANCE_BEFORE_REVERSE" != "null" ] && [ "$STAR_BALANCE_BEFORE_REVERSE" != "empty" ]; then
                    echo ">>> STAR Balance BEFORE reverse swap: $STAR_BALANCE_BEFORE_REVERSE"
                else
                    echo ">>> STAR Balance BEFORE reverse swap: Could not extract balance"
                    STAR_BALANCE_BEFORE_REVERSE="0"
                fi
                
                # Query price to estimate STAR needed for target eSOL amount
                echo ""
                echo "Querying price to estimate STAR needed for $ESOL_TARGET_AMOUNT eSOL..."
                REVERSE_PRICE_ESTIMATE_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token1_for_token2_price "{\"token1_amount\": \"$ESOL_TARGET_AMOUNT\"}" 2>&1)
                echo "$REVERSE_PRICE_ESTIMATE_OUTPUT"
                
                # Extract estimated STAR amount needed
                if command -v jq &> /dev/null; then
                    STAR_ESTIMATE=$(echo "$REVERSE_PRICE_ESTIMATE_OUTPUT" | jq -r '.token2_amount // empty' 2>/dev/null)
                fi
                if [ -z "$STAR_ESTIMATE" ]; then
                    STAR_ESTIMATE=$(echo "$REVERSE_PRICE_ESTIMATE_OUTPUT" | grep -o '"token2_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token2_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$STAR_ESTIMATE" ] && [ "$STAR_ESTIMATE" != "null" ] && [ "$STAR_ESTIMATE" != "empty" ]; then
                    echo ">>> Estimated STAR needed: $STAR_ESTIMATE (using $STAR_ESTIMATE with 5% buffer)"
                    STAR_INPUT_AMOUNT=$(echo "scale=0; $STAR_ESTIMATE * 105 / 100" | bc 2>/dev/null || echo "$STAR_ESTIMATE")
                else
                    echo ">>> Could not extract price estimate, using default amount"
                    STAR_INPUT_AMOUNT="600000"  # Fallback amount
                fi
                
                # Query actual eSOL output for the calculated STAR input
                echo ""
                echo "Querying actual eSOL output for $STAR_INPUT_AMOUNT STAR input..."
                REVERSE_ACTUAL_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token2_for_token1_price "{\"token2_amount\": \"$STAR_INPUT_AMOUNT\"}" 2>&1)
                echo "$REVERSE_ACTUAL_OUTPUT"
                
                # Extract actual eSOL output
                if command -v jq &> /dev/null; then
                    ESOL_ACTUAL_OUTPUT=$(echo "$REVERSE_ACTUAL_OUTPUT" | jq -r '.token1_amount // empty' 2>/dev/null)
                fi
                if [ -z "$ESOL_ACTUAL_OUTPUT" ]; then
                    ESOL_ACTUAL_OUTPUT=$(echo "$REVERSE_ACTUAL_OUTPUT" | grep -o '"token1_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token1_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$ESOL_ACTUAL_OUTPUT" ] && [ "$ESOL_ACTUAL_OUTPUT" != "null" ] && [ "$ESOL_ACTUAL_OUTPUT" != "empty" ]; then
                    echo ">>> Actual eSOL output for $STAR_INPUT_AMOUNT STAR: $ESOL_ACTUAL_OUTPUT"
                    # Set min_output to 95% of actual output for safety
                    MIN_ESOL_OUTPUT=$(echo "scale=0; $ESOL_ACTUAL_OUTPUT * 95 / 100" | bc 2>/dev/null || echo "$ESOL_ACTUAL_OUTPUT")
                    echo ">>> Setting min_output to $MIN_ESOL_OUTPUT (95% of actual output for safety)"
                else
                    echo ">>> Could not extract actual output, using conservative min_output"
                    MIN_ESOL_OUTPUT=$(echo "scale=0; $ESOL_TARGET_AMOUNT * 90 / 100" | bc 2>/dev/null || echo "$ESOL_TARGET_AMOUNT")
                fi
                
                # Check STAR allowance for AMM
                echo ""
                echo "Checking current STAR allowance for AMM contract..."
                STAR_ALLOWANCE_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" allowance "{\"owner\": \"$WALLET1_ADDRESS\", \"spender\": \"$AMM_CONTRACT_ID\"}" 2>&1)
                echo "$STAR_ALLOWANCE_OUTPUT"
                
                # Extract current allowance
                if command -v jq &> /dev/null; then
                    CURRENT_STAR_ALLOWANCE=$(echo "$STAR_ALLOWANCE_OUTPUT" | jq -r '.allowance // empty' 2>/dev/null)
                fi
                if [ -z "$CURRENT_STAR_ALLOWANCE" ]; then
                    CURRENT_STAR_ALLOWANCE=$(echo "$STAR_ALLOWANCE_OUTPUT" | grep -o '"allowance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"allowance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -z "$CURRENT_STAR_ALLOWANCE" ] || [ "$CURRENT_STAR_ALLOWANCE" = "null" ] || [ "$CURRENT_STAR_ALLOWANCE" = "empty" ]; then
                    CURRENT_STAR_ALLOWANCE="0"
                fi
                echo ">>> Current STAR allowance: $CURRENT_STAR_ALLOWANCE"
                
                # Add 5% buffer to the required amount
                REQUIRED_STAR_ALLOWANCE=$(echo "scale=0; $STAR_INPUT_AMOUNT * 105 / 100" | bc 2>/dev/null || echo "$STAR_INPUT_AMOUNT")
                
                if [ "$CURRENT_STAR_ALLOWANCE" -lt "$REQUIRED_STAR_ALLOWANCE" ] 2>/dev/null; then
                    echo ""
                    echo "Current allowance ($CURRENT_STAR_ALLOWANCE) is insufficient. Increasing allowance to $REQUIRED_STAR_ALLOWANCE..."
                    STAR_ALLOWANCE_TO_ADD=$(echo "scale=0; $REQUIRED_STAR_ALLOWANCE - $CURRENT_STAR_ALLOWANCE" | bc 2>/dev/null || echo "$REQUIRED_STAR_ALLOWANCE")
                    STAR_ALLOWANCE_OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$STAR_CONTRACT_ID" increase_allowance "{\"spender\": \"$AMM_CONTRACT_ID\", \"amount\": \"$STAR_ALLOWANCE_TO_ADD\", \"expires\": null}" 2>&1)
                    write_tx_response "star_increase_allowance_reverse_swap" "$STAR_ALLOWANCE_OUTPUT"
                    EXIT_CODE=$?
                    echo "$STAR_ALLOWANCE_OUTPUT"
                    
                    if [ $EXIT_CODE -eq 0 ] && echo "$STAR_ALLOWANCE_OUTPUT" | grep -qi "Contract execution transaction was sent successfully"; then
                        echo "STAR allowance increased successfully"
                        sleep 3
                    else
                        echo "WARNING: Setting STAR allowance may have failed, but continuing with swap attempt..."
                        sleep 2
                    fi
                else
                    echo ">>> Current allowance ($CURRENT_STAR_ALLOWANCE) is sufficient for reverse swap (need $REQUIRED_STAR_ALLOWANCE)"
                fi
                
                # Execute the reverse swap with retry logic for nonce errors
                echo ""
                echo "Executing reverse swap: Buying eSOL with STAR..."
                echo "Input: $STAR_INPUT_AMOUNT STAR (Token2)"
                echo "Target: ~$ESOL_TARGET_AMOUNT eSOL (Token1)"
                echo "Minimum output: $MIN_ESOL_OUTPUT eSOL (Token1) - protects against slippage"
                
                MAX_RETRIES=3
                RETRY_DELAY=3
                REVERSE_SWAP_SUCCESS=false
                
                for RETRY_ATTEMPT in $(seq 0 $MAX_RETRIES); do
                    if [ $RETRY_ATTEMPT -gt 0 ]; then
                        echo ""
                        echo "Retry attempt $RETRY_ATTEMPT/$MAX_RETRIES: Waiting $RETRY_DELAY seconds before retry..."
                        sleep $RETRY_DELAY
                    fi
                    
                    REVERSE_SWAP_OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$AMM_CONTRACT_ID" swap "{\"input_token\": \"Token2\", \"input_amount\": \"$STAR_INPUT_AMOUNT\", \"min_output\": \"$MIN_ESOL_OUTPUT\", \"expiration\": null}" 2>&1)
                    write_tx_response "amm_swap_star_to_esol" "$REVERSE_SWAP_OUTPUT"
                    EXIT_CODE=$?
                    echo "$REVERSE_SWAP_OUTPUT"
                    
                    # Check if it's a nonce error
                    if echo "$REVERSE_SWAP_OUTPUT" | grep -qi "code 3\|Nonce is not sequential\|First nonce should be 0"; then
                        if [ $RETRY_ATTEMPT -lt $MAX_RETRIES ]; then
                            echo "WARNING: Nonce error detected. Will retry after waiting..."
                            continue
                        else
                            echo "ERROR: Nonce error after $MAX_RETRIES retries. Giving up."
                            break
                        fi
                    fi
                    
                    # Check for success
                    if [ $EXIT_CODE -eq 0 ] && echo "$REVERSE_SWAP_OUTPUT" | grep -qi "successfully\|sent successfully"; then
                        REVERSE_SWAP_SUCCESS=true
                        break
                    fi
                    
                    # Check for other errors
                    if [ $EXIT_CODE -ne 0 ] || echo "$REVERSE_SWAP_OUTPUT" | grep -qi "FAILED\|Error\|error\|rejected\|invalid"; then
                        if [ $RETRY_ATTEMPT -lt $MAX_RETRIES ]; then
                            echo "WARNING: Transaction failed. Will retry after waiting..."
                            continue
                        else
                            echo "ERROR: Reverse swap transaction failed after $MAX_RETRIES retries."
                            break
                        fi
                    fi
                done
                
                if [ "$REVERSE_SWAP_SUCCESS" = true ]; then
                    echo "Reverse swap transaction sent successfully"
                    # Wait for transaction to be processed
                    sleep 5
                    
                    # Query balances after reverse swap
                    echo ""
                    echo "=========================================="
                    echo "Querying balances AFTER reverse swap..."
                    echo "=========================================="
                    echo ""
                    echo "Querying eSOL balance..."
                    ESOL_BALANCE_AFTER_REVERSE_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                    echo "$ESOL_BALANCE_AFTER_REVERSE_OUTPUT"
                    
                    # Extract eSOL balance
                    if command -v jq &> /dev/null; then
                        ESOL_BALANCE_AFTER_REVERSE=$(echo "$ESOL_BALANCE_AFTER_REVERSE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                    fi
                    if [ -z "$ESOL_BALANCE_AFTER_REVERSE" ]; then
                        ESOL_BALANCE_AFTER_REVERSE=$(echo "$ESOL_BALANCE_AFTER_REVERSE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -n "$ESOL_BALANCE_AFTER_REVERSE" ] && [ "$ESOL_BALANCE_AFTER_REVERSE" != "null" ] && [ "$ESOL_BALANCE_AFTER_REVERSE" != "empty" ]; then
                        echo ">>> eSOL Balance AFTER reverse swap: $ESOL_BALANCE_AFTER_REVERSE"
                    else
                        echo ">>> eSOL Balance AFTER reverse swap: Could not extract balance"
                        ESOL_BALANCE_AFTER_REVERSE="0"
                    fi
                    
                    echo ""
                    echo "Querying STAR balance..."
                    STAR_BALANCE_AFTER_REVERSE_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                    echo "$STAR_BALANCE_AFTER_REVERSE_OUTPUT"
                    
                    # Extract STAR balance
                    if command -v jq &> /dev/null; then
                        STAR_BALANCE_AFTER_REVERSE=$(echo "$STAR_BALANCE_AFTER_REVERSE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                    fi
                    if [ -z "$STAR_BALANCE_AFTER_REVERSE" ]; then
                        STAR_BALANCE_AFTER_REVERSE=$(echo "$STAR_BALANCE_AFTER_REVERSE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -n "$STAR_BALANCE_AFTER_REVERSE" ] && [ "$STAR_BALANCE_AFTER_REVERSE" != "null" ] && [ "$STAR_BALANCE_AFTER_REVERSE" != "empty" ]; then
                        echo ">>> STAR Balance AFTER reverse swap: $STAR_BALANCE_AFTER_REVERSE"
                    else
                        echo ">>> STAR Balance AFTER reverse swap: Could not extract balance"
                        STAR_BALANCE_AFTER_REVERSE="0"
                    fi
                    
                    # Calculate changes
                    echo ""
                    echo "Balance Changes (reverse swap):"
                    if [ "$ESOL_BALANCE_BEFORE_REVERSE" != "0" ] && [ "$ESOL_BALANCE_AFTER_REVERSE" != "0" ]; then
                        ESOL_CHANGE_REVERSE=$(echo "scale=0; $ESOL_BALANCE_AFTER_REVERSE - $ESOL_BALANCE_BEFORE_REVERSE" | bc 2>/dev/null || echo "N/A")
                        echo ">>> eSOL Change: $ESOL_CHANGE_REVERSE"
                    fi
                    if [ "$STAR_BALANCE_BEFORE_REVERSE" != "0" ] && [ "$STAR_BALANCE_AFTER_REVERSE" != "0" ]; then
                        STAR_CHANGE_REVERSE=$(echo "scale=0; $STAR_BALANCE_AFTER_REVERSE - $STAR_BALANCE_BEFORE_REVERSE" | bc 2>/dev/null || echo "N/A")
                        echo ">>> STAR Change: $STAR_CHANGE_REVERSE"
                    fi
                    
                    # Query prices after reverse swap
                    echo ""
                    echo "=========================================="
                    echo "Querying prices AFTER reverse swap..."
                    echo "=========================================="
                    echo ""
                    echo "Querying eSOL price (how much STAR for $PRICE_QUERY_AMOUNT eSOL)..."
                    TOKEN1_PRICE_AFTER_REVERSE_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token1_for_token2_price "{\"token1_amount\": \"$PRICE_QUERY_AMOUNT\"}" 2>&1)
                    echo "$TOKEN1_PRICE_AFTER_REVERSE_OUTPUT"
                    
                    # Extract token2_amount from the response
                    if command -v jq &> /dev/null; then
                        TOKEN2_AMOUNT_AFTER_REVERSE=$(echo "$TOKEN1_PRICE_AFTER_REVERSE_OUTPUT" | jq -r '.token2_amount // empty' 2>/dev/null)
                    fi
                    if [ -z "$TOKEN2_AMOUNT_AFTER_REVERSE" ]; then
                        TOKEN2_AMOUNT_AFTER_REVERSE=$(echo "$TOKEN1_PRICE_AFTER_REVERSE_OUTPUT" | grep -o '"token2_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token2_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -n "$TOKEN2_AMOUNT_AFTER_REVERSE" ] && [ "$TOKEN2_AMOUNT_AFTER_REVERSE" != "null" ] && [ "$TOKEN2_AMOUNT_AFTER_REVERSE" != "empty" ]; then
                        echo ">>> eSOL Price AFTER reverse swap: $PRICE_QUERY_AMOUNT eSOL = $TOKEN2_AMOUNT_AFTER_REVERSE STAR"
                    else
                        echo ">>> eSOL Price AFTER reverse swap: Could not extract price from response"
                    fi
                    
                    echo ""
                    echo "Querying STAR price (how much eSOL for $PRICE_QUERY_AMOUNT STAR)..."
                    TOKEN2_PRICE_AFTER_REVERSE_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" token2_for_token1_price "{\"token2_amount\": \"$PRICE_QUERY_AMOUNT\"}" 2>&1)
                    echo "$TOKEN2_PRICE_AFTER_REVERSE_OUTPUT"
                    
                    # Extract token1_amount from the response
                    if command -v jq &> /dev/null; then
                        TOKEN1_AMOUNT_AFTER_REVERSE=$(echo "$TOKEN2_PRICE_AFTER_REVERSE_OUTPUT" | jq -r '.token1_amount // empty' 2>/dev/null)
                    fi
                    if [ -z "$TOKEN1_AMOUNT_AFTER_REVERSE" ]; then
                        TOKEN1_AMOUNT_AFTER_REVERSE=$(echo "$TOKEN2_PRICE_AFTER_REVERSE_OUTPUT" | grep -o '"token1_amount"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token1_amount"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -n "$TOKEN1_AMOUNT_AFTER_REVERSE" ] && [ "$TOKEN1_AMOUNT_AFTER_REVERSE" != "null" ] && [ "$TOKEN1_AMOUNT_AFTER_REVERSE" != "empty" ]; then
                        echo ">>> STAR Price AFTER reverse swap: $PRICE_QUERY_AMOUNT STAR = $TOKEN1_AMOUNT_AFTER_REVERSE eSOL"
                    else
                        echo ">>> STAR Price AFTER reverse swap: Could not extract price from response"
                    fi
                    
                    echo ""
                    echo "=========================================="
                else
                    echo "WARNING: Could not determine reverse swap transaction status"
                fi
                
                # ==========================================
                # Remove Liquidity: Partially remove liquidity
                # ==========================================
                echo ""
                echo "=========================================="
                echo "=========================================="
                echo "Removing partial liquidity (~50%)"
                echo "=========================================="
                echo "=========================================="
                
                # Query balances BEFORE remove liquidity
                echo ""
                echo "Querying balances BEFORE remove liquidity..."
                echo "Querying LP token balance..."
                echo "LP_TOKEN_ADDRESS: $LP_TOKEN_ADDRESS"
                LP_BALANCE_BEFORE_REMOVE_OUTPUT=$(cargo run -- contract-query2 "$LP_TOKEN_ADDRESS" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                echo "$LP_BALANCE_BEFORE_REMOVE_OUTPUT"
                
                # Extract LP token balance
                if command -v jq &> /dev/null; then
                    LP_BALANCE_BEFORE_REMOVE=$(echo "$LP_BALANCE_BEFORE_REMOVE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                fi
                if [ -z "$LP_BALANCE_BEFORE_REMOVE" ]; then
                    LP_BALANCE_BEFORE_REMOVE=$(echo "$LP_BALANCE_BEFORE_REMOVE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$LP_BALANCE_BEFORE_REMOVE" ] && [ "$LP_BALANCE_BEFORE_REMOVE" != "null" ] && [ "$LP_BALANCE_BEFORE_REMOVE" != "empty" ]; then
                    echo ">>> LP Token Balance BEFORE: $LP_BALANCE_BEFORE_REMOVE"
                else
                    echo ">>> LP Token Balance BEFORE: Could not extract balance"
                    LP_BALANCE_BEFORE_REMOVE="0"
                fi
                
                echo ""
                echo "Querying eSOL balance..."
                ESOL_BALANCE_BEFORE_REMOVE_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                echo "$ESOL_BALANCE_BEFORE_REMOVE_OUTPUT"
                
                # Extract eSOL balance
                if command -v jq &> /dev/null; then
                    ESOL_BALANCE_BEFORE_REMOVE=$(echo "$ESOL_BALANCE_BEFORE_REMOVE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                fi
                if [ -z "$ESOL_BALANCE_BEFORE_REMOVE" ]; then
                    ESOL_BALANCE_BEFORE_REMOVE=$(echo "$ESOL_BALANCE_BEFORE_REMOVE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$ESOL_BALANCE_BEFORE_REMOVE" ] && [ "$ESOL_BALANCE_BEFORE_REMOVE" != "null" ] && [ "$ESOL_BALANCE_BEFORE_REMOVE" != "empty" ]; then
                    echo ">>> eSOL Balance BEFORE: $ESOL_BALANCE_BEFORE_REMOVE"
                else
                    echo ">>> eSOL Balance BEFORE: Could not extract balance"
                    ESOL_BALANCE_BEFORE_REMOVE="0"
                fi
                
                echo ""
                echo "Querying STAR balance..."
                STAR_BALANCE_BEFORE_REMOVE_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                echo "$STAR_BALANCE_BEFORE_REMOVE_OUTPUT"
                
                # Extract STAR balance
                if command -v jq &> /dev/null; then
                    STAR_BALANCE_BEFORE_REMOVE=$(echo "$STAR_BALANCE_BEFORE_REMOVE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                fi
                if [ -z "$STAR_BALANCE_BEFORE_REMOVE" ]; then
                    STAR_BALANCE_BEFORE_REMOVE=$(echo "$STAR_BALANCE_BEFORE_REMOVE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                fi
                if [ -n "$STAR_BALANCE_BEFORE_REMOVE" ] && [ "$STAR_BALANCE_BEFORE_REMOVE" != "null" ] && [ "$STAR_BALANCE_BEFORE_REMOVE" != "empty" ]; then
                    echo ">>> STAR Balance BEFORE: $STAR_BALANCE_BEFORE_REMOVE"
                else
                    echo ">>> STAR Balance BEFORE: Could not extract balance"
                    STAR_BALANCE_BEFORE_REMOVE="0"
                fi
                
                # Calculate LP tokens to burn (around 50%)
                if [ "$LP_BALANCE_BEFORE_REMOVE" != "0" ] && [ -n "$LP_BALANCE_BEFORE_REMOVE" ]; then
                    LP_TO_BURN=$(echo "scale=0; $LP_BALANCE_BEFORE_REMOVE / 2" | bc 2>/dev/null || echo "$LP_BALANCE_BEFORE_REMOVE")
                    echo ""
                    echo ">>> LP Tokens to burn: $LP_TO_BURN (approximately 50% of $LP_BALANCE_BEFORE_REMOVE)"
                    
                    # Query AMM info to get current reserves and LP supply
                    echo ""
                    echo "Querying AMM info to calculate expected token amounts..."
                    AMM_INFO_REMOVE_OUTPUT=$(cargo run -- contract-query2 "$AMM_CONTRACT_ID" info '{}' 2>&1)
                    echo "$AMM_INFO_REMOVE_OUTPUT"
                    
                    # Extract reserves and LP supply
                    if command -v jq &> /dev/null; then
                        TOKEN1_RESERVE=$(echo "$AMM_INFO_REMOVE_OUTPUT" | jq -r '.token1_reserve // empty' 2>/dev/null)
                        TOKEN2_RESERVE=$(echo "$AMM_INFO_REMOVE_OUTPUT" | jq -r '.token2_reserve // empty' 2>/dev/null)
                        LP_SUPPLY=$(echo "$AMM_INFO_REMOVE_OUTPUT" | jq -r '.lp_token_supply // empty' 2>/dev/null)
                    fi
                    if [ -z "$TOKEN1_RESERVE" ]; then
                        TOKEN1_RESERVE=$(echo "$AMM_INFO_REMOVE_OUTPUT" | grep -o '"token1_reserve"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token1_reserve"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -z "$TOKEN2_RESERVE" ]; then
                        TOKEN2_RESERVE=$(echo "$AMM_INFO_REMOVE_OUTPUT" | grep -o '"token2_reserve"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"token2_reserve"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    if [ -z "$LP_SUPPLY" ]; then
                        LP_SUPPLY=$(echo "$AMM_INFO_REMOVE_OUTPUT" | grep -o '"lp_token_supply"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"lp_token_supply"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                    fi
                    
                    if [ -n "$TOKEN1_RESERVE" ] && [ -n "$TOKEN2_RESERVE" ] && [ -n "$LP_SUPPLY" ] && \
                       [ "$TOKEN1_RESERVE" != "null" ] && [ "$TOKEN2_RESERVE" != "null" ] && [ "$LP_SUPPLY" != "null" ] && \
                       [ "$TOKEN1_RESERVE" != "empty" ] && [ "$TOKEN2_RESERVE" != "empty" ] && [ "$LP_SUPPLY" != "empty" ]; then
                        echo ">>> Current reserves: Token1 (eSOL) = $TOKEN1_RESERVE, Token2 (STAR) = $TOKEN2_RESERVE"
                        echo ">>> Current LP supply: $LP_SUPPLY"
                        
                        # Calculate expected token amounts based on proportion
                        # Expected = (LP_to_burn / LP_supply) * reserve
                        EXPECTED_TOKEN1=$(echo "scale=0; ($LP_TO_BURN * $TOKEN1_RESERVE) / $LP_SUPPLY" | bc 2>/dev/null || echo "0")
                        EXPECTED_TOKEN2=$(echo "scale=0; ($LP_TO_BURN * $TOKEN2_RESERVE) / $LP_SUPPLY" | bc 2>/dev/null || echo "0")
                        
                        # Set min amounts to 95% of expected for safety
                        MIN_TOKEN1=$(echo "scale=0; $EXPECTED_TOKEN1 * 95 / 100" | bc 2>/dev/null || echo "$EXPECTED_TOKEN1")
                        MIN_TOKEN2=$(echo "scale=0; $EXPECTED_TOKEN2 * 95 / 100" | bc 2>/dev/null || echo "$EXPECTED_TOKEN2")
                        
                        echo ">>> Expected Token1 (eSOL): $EXPECTED_TOKEN1"
                        echo ">>> Expected Token2 (STAR): $EXPECTED_TOKEN2"
                        echo ">>> Min Token1 (eSOL): $MIN_TOKEN1 (95% of expected)"
                        echo ">>> Min Token2 (STAR): $MIN_TOKEN2 (95% of expected)"
                        
                        # Check LP token allowance for AMM
                        echo ""
                        echo "Checking LP token allowance for AMM contract..."
                        echo "LP_TOKEN_ADDRESS: $LP_TOKEN_ADDRESS"
                        LP_ALLOWANCE_OUTPUT=$(cargo run -- contract-query2 "$LP_TOKEN_ADDRESS" allowance "{\"owner\": \"$WALLET1_ADDRESS\", \"spender\": \"$AMM_CONTRACT_ID\"}" 2>&1)
                        echo "$LP_ALLOWANCE_OUTPUT"
                        
                        # Extract current allowance
                        if command -v jq &> /dev/null; then
                            CURRENT_LP_ALLOWANCE=$(echo "$LP_ALLOWANCE_OUTPUT" | jq -r '.allowance // empty' 2>/dev/null)
                        fi
                        if [ -z "$CURRENT_LP_ALLOWANCE" ]; then
                            CURRENT_LP_ALLOWANCE=$(echo "$LP_ALLOWANCE_OUTPUT" | grep -o '"allowance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"allowance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                        fi
                        if [ -z "$CURRENT_LP_ALLOWANCE" ] || [ "$CURRENT_LP_ALLOWANCE" = "null" ] || [ "$CURRENT_LP_ALLOWANCE" = "empty" ]; then
                            CURRENT_LP_ALLOWANCE="0"
                        fi
                        echo ">>> Current LP token allowance: $CURRENT_LP_ALLOWANCE"
                        
                        # Add 5% buffer to the required amount
                        REQUIRED_LP_ALLOWANCE=$(echo "scale=0; $LP_TO_BURN * 105 / 100" | bc 2>/dev/null || echo "$LP_TO_BURN")
                        
                        if [ "$CURRENT_LP_ALLOWANCE" -lt "$REQUIRED_LP_ALLOWANCE" ] 2>/dev/null; then
                            echo ""
                            echo "Current allowance ($CURRENT_LP_ALLOWANCE) is insufficient. Increasing allowance to $REQUIRED_LP_ALLOWANCE..."
                            LP_ALLOWANCE_TO_ADD=$(echo "scale=0; $REQUIRED_LP_ALLOWANCE - $CURRENT_LP_ALLOWANCE" | bc 2>/dev/null || echo "$REQUIRED_LP_ALLOWANCE")
                            LP_ALLOWANCE_TX_OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$LP_TOKEN_ADDRESS" increase_allowance "{\"spender\": \"$AMM_CONTRACT_ID\", \"amount\": \"$LP_ALLOWANCE_TO_ADD\", \"expires\": null}" 2>&1)
                            write_tx_response "lp_token_increase_allowance" "$LP_ALLOWANCE_TX_OUTPUT"
                            EXIT_CODE=$?
                            echo "$LP_ALLOWANCE_TX_OUTPUT"
                            
                            if [ $EXIT_CODE -eq 0 ] && echo "$LP_ALLOWANCE_TX_OUTPUT" | grep -qi "Contract execution transaction was sent successfully"; then
                                echo "LP token allowance increased successfully"
                                sleep 3
                            else
                                echo "WARNING: Setting LP token allowance may have failed, but continuing with remove_liquidity attempt..."
                                sleep 2
                            fi
                        else
                            echo ">>> Current allowance ($CURRENT_LP_ALLOWANCE) is sufficient for remove_liquidity (need $REQUIRED_LP_ALLOWANCE)"
                        fi
                        
                        # Execute remove_liquidity with retry logic for nonce errors
                        echo ""
                        echo "Executing remove_liquidity..."
                        echo "LP Tokens to burn: $LP_TO_BURN"
                        echo "Min Token1 (eSOL): $MIN_TOKEN1"
                        echo "Min Token2 (STAR): $MIN_TOKEN2"
                        
                        MAX_RETRIES=3
                        RETRY_DELAY=3
                        REMOVE_LIQUIDITY_SUCCESS=false
                        
                        for RETRY_ATTEMPT in $(seq 0 $MAX_RETRIES); do
                            if [ $RETRY_ATTEMPT -gt 0 ]; then
                                echo ""
                                echo "Retry attempt $RETRY_ATTEMPT/$MAX_RETRIES: Waiting $RETRY_DELAY seconds before retry..."
                                sleep $RETRY_DELAY
                            fi
                            
                            REMOVE_LIQUIDITY_OUTPUT=$(cargo run -- execute-contract "$DEV_WALLET" "$AMM_CONTRACT_ID" remove_liquidity "{\"amount\": \"$LP_TO_BURN\", \"min_token1\": \"$MIN_TOKEN1\", \"min_token2\": \"$MIN_TOKEN2\", \"expiration\": null}" 2>&1)
                            write_tx_response "amm_remove_liquidity" "$REMOVE_LIQUIDITY_OUTPUT"
                            EXIT_CODE=$?
                            echo "$REMOVE_LIQUIDITY_OUTPUT"
                            
                            # Check if it's a nonce error
                            if echo "$REMOVE_LIQUIDITY_OUTPUT" | grep -qi "code 3\|Nonce is not sequential\|First nonce should be 0"; then
                                if [ $RETRY_ATTEMPT -lt $MAX_RETRIES ]; then
                                    echo "WARNING: Nonce error detected. Will retry after waiting..."
                                    continue
                                else
                                    echo "ERROR: Nonce error after $MAX_RETRIES retries. Giving up."
                                    break
                                fi
                            fi
                            
                            # Check for success
                            if [ $EXIT_CODE -eq 0 ] && echo "$REMOVE_LIQUIDITY_OUTPUT" | grep -qi "successfully\|sent successfully"; then
                                REMOVE_LIQUIDITY_SUCCESS=true
                                break
                            fi
                            
                            # Check for other errors
                            if [ $EXIT_CODE -ne 0 ] || echo "$REMOVE_LIQUIDITY_OUTPUT" | grep -qi "FAILED\|Error\|error\|rejected\|invalid"; then
                                if [ $RETRY_ATTEMPT -lt $MAX_RETRIES ]; then
                                    echo "WARNING: Transaction failed. Will retry after waiting..."
                                    continue
                                else
                                    echo "ERROR: Remove liquidity transaction failed after $MAX_RETRIES retries."
                                    break
                                fi
                            fi
                        done
                        
                        if [ "$REMOVE_LIQUIDITY_SUCCESS" = true ]; then
                            echo "Remove liquidity transaction sent successfully"
                            # Wait for transaction to be processed
                            sleep 5
                            
                            # Query balances after remove liquidity
                            echo ""
                            echo "=========================================="
                            echo "Querying balances AFTER remove liquidity..."
                            echo "=========================================="
                            echo ""
                            echo "Querying LP token balance..."
                            echo "LP_TOKEN_ADDRESS: $LP_TOKEN_ADDRESS"
                            LP_BALANCE_AFTER_REMOVE_OUTPUT=$(cargo run -- contract-query2 "$LP_TOKEN_ADDRESS" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                            echo "$LP_BALANCE_AFTER_REMOVE_OUTPUT"
                            
                            # Extract LP token balance
                            if command -v jq &> /dev/null; then
                                LP_BALANCE_AFTER_REMOVE=$(echo "$LP_BALANCE_AFTER_REMOVE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                            fi
                            if [ -z "$LP_BALANCE_AFTER_REMOVE" ]; then
                                LP_BALANCE_AFTER_REMOVE=$(echo "$LP_BALANCE_AFTER_REMOVE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                            fi
                            if [ -n "$LP_BALANCE_AFTER_REMOVE" ] && [ "$LP_BALANCE_AFTER_REMOVE" != "null" ] && [ "$LP_BALANCE_AFTER_REMOVE" != "empty" ]; then
                                echo ">>> LP Token Balance AFTER: $LP_BALANCE_AFTER_REMOVE"
                            else
                                echo ">>> LP Token Balance AFTER: Could not extract balance"
                                LP_BALANCE_AFTER_REMOVE="0"
                            fi
                            
                            echo ""
                            echo "Querying eSOL balance..."
                            ESOL_BALANCE_AFTER_REMOVE_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                            echo "$ESOL_BALANCE_AFTER_REMOVE_OUTPUT"
                            
                            # Extract eSOL balance
                            if command -v jq &> /dev/null; then
                                ESOL_BALANCE_AFTER_REMOVE=$(echo "$ESOL_BALANCE_AFTER_REMOVE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                            fi
                            if [ -z "$ESOL_BALANCE_AFTER_REMOVE" ]; then
                                ESOL_BALANCE_AFTER_REMOVE=$(echo "$ESOL_BALANCE_AFTER_REMOVE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                            fi
                            if [ -n "$ESOL_BALANCE_AFTER_REMOVE" ] && [ "$ESOL_BALANCE_AFTER_REMOVE" != "null" ] && [ "$ESOL_BALANCE_AFTER_REMOVE" != "empty" ]; then
                                echo ">>> eSOL Balance AFTER: $ESOL_BALANCE_AFTER_REMOVE"
                            else
                                echo ">>> eSOL Balance AFTER: Could not extract balance"
                                ESOL_BALANCE_AFTER_REMOVE="0"
                            fi
                            
                            echo ""
                            echo "Querying STAR balance..."
                            STAR_BALANCE_AFTER_REMOVE_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" balance "{\"address\": \"$WALLET1_ADDRESS\"}" 2>&1)
                            echo "$STAR_BALANCE_AFTER_REMOVE_OUTPUT"
                            
                            # Extract STAR balance
                            if command -v jq &> /dev/null; then
                                STAR_BALANCE_AFTER_REMOVE=$(echo "$STAR_BALANCE_AFTER_REMOVE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
                            fi
                            if [ -z "$STAR_BALANCE_AFTER_REMOVE" ]; then
                                STAR_BALANCE_AFTER_REMOVE=$(echo "$STAR_BALANCE_AFTER_REMOVE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
                            fi
                            if [ -n "$STAR_BALANCE_AFTER_REMOVE" ] && [ "$STAR_BALANCE_AFTER_REMOVE" != "null" ] && [ "$STAR_BALANCE_AFTER_REMOVE" != "empty" ]; then
                                echo ">>> STAR Balance AFTER: $STAR_BALANCE_AFTER_REMOVE"
                            else
                                echo ">>> STAR Balance AFTER: Could not extract balance"
                                STAR_BALANCE_AFTER_REMOVE="0"
                            fi
                            
                            # Calculate changes
                            echo ""
                            echo "Balance Changes (remove liquidity):"
                            if [ "$LP_BALANCE_BEFORE_REMOVE" != "0" ] && [ "$LP_BALANCE_AFTER_REMOVE" != "0" ]; then
                                LP_CHANGE=$(echo "scale=0; $LP_BALANCE_AFTER_REMOVE - $LP_BALANCE_BEFORE_REMOVE" | bc 2>/dev/null || echo "N/A")
                                echo ">>> LP Token Change: $LP_CHANGE"
                            fi
                            if [ "$ESOL_BALANCE_BEFORE_REMOVE" != "0" ] && [ "$ESOL_BALANCE_AFTER_REMOVE" != "0" ]; then
                                ESOL_CHANGE_REMOVE=$(echo "scale=0; $ESOL_BALANCE_AFTER_REMOVE - $ESOL_BALANCE_BEFORE_REMOVE" | bc 2>/dev/null || echo "N/A")
                                echo ">>> eSOL Change: $ESOL_CHANGE_REMOVE"
                            fi
                            if [ "$STAR_BALANCE_BEFORE_REMOVE" != "0" ] && [ "$STAR_BALANCE_AFTER_REMOVE" != "0" ]; then
                                STAR_CHANGE_REMOVE=$(echo "scale=0; $STAR_BALANCE_AFTER_REMOVE - $STAR_BALANCE_BEFORE_REMOVE" | bc 2>/dev/null || echo "N/A")
                                echo ">>> STAR Change: $STAR_CHANGE_REMOVE"
                            fi
                            
                            echo ""
                            echo "=========================================="
                        else
                            echo "WARNING: Could not determine remove_liquidity transaction status"
                        fi
                    else
                        echo ">>> WARNING: Could not extract AMM reserves/LP supply. Cannot calculate min amounts."
                    fi
                else
                    echo ">>> WARNING: Cannot determine LP token balance. Skipping remove_liquidity."
                fi
        fi
    else
        echo "WARNING: Could not determine transaction status from output"
        echo "Full output:"
        echo "$OUTPUT"
            fi
        else
            echo "ERROR: Final allowance verification failed. Cannot proceed with add_liquidity."
        fi
    else
        echo ""
        echo "ERROR: Cannot proceed with add_liquidity - one or more approvals failed"
        echo "eSOL approved: $ESOL_APPROVED"
        echo "STAR approved: $STAR_APPROVED"
        echo "Please check the approval transactions above and fix any issues before retrying."
    fi
else
    echo ""
    echo "Warning: Could not extract AMM contract_id from deployment file. Skipping AMM approval and liquidity transactions."
fi

# Return to original directory
cd ../eld_node_app || exit 1

echo "Contract deployments completed (eSOL, STAR, BasicReceiver, and AMM). App is running (PID: $APP_PID)"
echo "Press Ctrl+C to stop the app"

# Query and display final AMM contract balances
if [ -n "$AMM_CONTRACT_ID" ] && [ "$AMM_CONTRACT_ID" != "null" ] && [ "$AMM_CONTRACT_ID" != "empty" ]; then
    echo ""
    echo "=========================================="
    echo "Final AMM Contract Balances:"
    echo "=========================================="
    echo ""
    # Change to cli directory to run contract queries
    cd ../../clients/cli || exit 1
    echo "Querying AMM's eSOL balance..."
    FINAL_AMM_ESOL_BALANCE_OUTPUT=$(cargo run -- contract-query2 "$ESOL_CONTRACT_ID" balance "{\"address\": \"$AMM_CONTRACT_ID\"}" 2>&1)
    echo "$FINAL_AMM_ESOL_BALANCE_OUTPUT"
    
    # Extract eSOL balance
    if command -v jq &> /dev/null; then
        FINAL_AMM_ESOL_BALANCE=$(echo "$FINAL_AMM_ESOL_BALANCE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
    fi
    if [ -z "$FINAL_AMM_ESOL_BALANCE" ]; then
        FINAL_AMM_ESOL_BALANCE=$(echo "$FINAL_AMM_ESOL_BALANCE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
    fi
    if [ -n "$FINAL_AMM_ESOL_BALANCE" ] && [ "$FINAL_AMM_ESOL_BALANCE" != "null" ] && [ "$FINAL_AMM_ESOL_BALANCE" != "empty" ]; then
        echo ">>> AMM eSOL Balance: $FINAL_AMM_ESOL_BALANCE"
    else
        echo ">>> AMM eSOL Balance: Could not extract balance"
    fi
    
    echo ""
    echo "Querying AMM's STAR balance..."
    FINAL_AMM_STAR_BALANCE_OUTPUT=$(cargo run -- contract-query2 "$STAR_CONTRACT_ID" balance "{\"address\": \"$AMM_CONTRACT_ID\"}" 2>&1)
    echo "$FINAL_AMM_STAR_BALANCE_OUTPUT"
    
    # Extract STAR balance
    if command -v jq &> /dev/null; then
        FINAL_AMM_STAR_BALANCE=$(echo "$FINAL_AMM_STAR_BALANCE_OUTPUT" | jq -r '.balance // empty' 2>/dev/null)
    fi
    if [ -z "$FINAL_AMM_STAR_BALANCE" ]; then
        FINAL_AMM_STAR_BALANCE=$(echo "$FINAL_AMM_STAR_BALANCE_OUTPUT" | grep -o '"balance"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"balance"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
    fi
    if [ -n "$FINAL_AMM_STAR_BALANCE" ] && [ "$FINAL_AMM_STAR_BALANCE" != "null" ] && [ "$FINAL_AMM_STAR_BALANCE" != "empty" ]; then
        echo ">>> AMM STAR Balance: $FINAL_AMM_STAR_BALANCE"
    else
        echo ">>> AMM STAR Balance: Could not extract balance"
    fi
    
    echo ""
    echo "=========================================="
    # Return to eld_node_app directory
    cd ../eld_node_app || exit 1
fi

# Wait for the app process to keep the script alive
# This allows Ctrl+C to be caught by the signal handler
wait $APP_PID