#!/bin/bash

set -e  # Exit on error

echo "=== Cetus Transaction Indexer ==="
echo "This script will process Sui blockchain checkpoints and extract Cetus transactions."

# Create checkpoint directory if it doesn't exist
CHECKPOINTS_DIR=${CHECKPOINTS_DIR:-/home/hungez/Documents/suins-indexer-1/checkpoints}
echo "Creating checkpoints directory: $CHECKPOINTS_DIR"
mkdir -p $CHECKPOINTS_DIR

# Set environment variables
export CHECKPOINTS_DIR=/home/hungez/Documents/suins-indexer-1/checkpoints
export BACKFILL_PROGRESS_FILE_PATH=${BACKFILL_PROGRESS_FILE_PATH:-/home/hungez/Documents/suins-indexer-1/backfill_progress/backfill_progress}

# Check for database URL
if [ -z "$DATABASE_URL" ]; then
  echo "WARNING: DATABASE_URL is not set. Using default value for development."
  export DATABASE_URL="postgres://user:password@localhost:5432/sui_indexer"
  echo "To set a custom database URL, run: export DATABASE_URL=\"your_connection_string\""
fi

# Check for remote storage
if [ -z "$REMOTE_STORAGE" ]; then
  echo "NOTE: REMOTE_STORAGE is not set. The indexer will only process local checkpoints."
  echo "      To fetch checkpoints from a remote source, set REMOTE_STORAGE environment variable."
  echo "      Example: export REMOTE_STORAGE=\"s3://sui-mainnet-data/checkpoints\""
  
  # Check if the checkpoint directory is empty
  if [ -z "$(ls -A $CHECKPOINTS_DIR)" ]; then
    echo "WARNING: The checkpoint directory is empty. No data will be processed."
    echo "         Please download checkpoints or set a remote storage endpoint."
  fi
fi

echo ""
echo "Starting the Cetus checkpoint processor..."
echo "Press Ctrl+C to stop the processor."
echo ""

# Run the Cetus checkpoint processor
cargo run --bin cetus_checkpoint_processor

# Note: To download checkpoints, you might need to set REMOTE_STORAGE
# Example: export REMOTE_STORAGE="s3://sui-mainnet-data/checkpoints"
# and ensure AWS credentials are properly configured 