#!/bin/bash

# Remove immutable flags from capacity files before deletion
# This is necessary because capacity files are set to immutable to prevent accidental overwrites

if [ -d "./data/capacity" ]; then
    echo "Removing immutable flags from capacity files..."
    
    # Handle Linux (chattr)
    if command -v chattr &> /dev/null; then
        find ./data/capacity -type f -name "*.dat" -exec chattr -i {} \; 2>/dev/null || true
        find ./data/capacity -type f -name "*.slots.json" -exec chattr -i {} \; 2>/dev/null || true
    fi
    
    # Handle macOS (chflags)
    if command -v chflags &> /dev/null; then
        find ./data/capacity -type f -name "*.dat" -exec chflags nouchg {} \; 2>/dev/null || true
        find ./data/capacity -type f -name "*.slots.json" -exec chflags nouchg {} \; 2>/dev/null || true
    fi
    
    echo "Immutable flags removed"
fi

# Now delete the data directory
rm -f -r ./data
