# Cetus Indexer for Sui

This project is designed to index transactions related to the Cetus protocol on the Sui blockchain. It uses the Sui data ingestion framework to process checkpoints and identify Cetus-related transactions.

## Project Structure

- `src/cetus_indexer.rs`: Contains the Cetus indexer implementation
- `src/bin/cetus_checkpoint_processor.rs`: CLI tool to process checkpoints and log Cetus transactions
- `run_cetus_processor.sh`: Shell script to run the Cetus checkpoint processor

## Getting Started

### Prerequisites

- Rust and Cargo
- PostgreSQL database (for storing indexed data)
- Access to Sui checkpoints (local or remote)

### Running the Cetus Indexer

1. Set up your environment:

```bash
# Configure your database
export DATABASE_URL="postgres://user:password@localhost:5432/sui_indexer"

# Configure checkpoint source (local)
export CHECKPOINTS_DIR="/path/to/checkpoints"

# Or use remote storage (optional)
export REMOTE_STORAGE="s3://sui-mainnet-data/checkpoints"
```

2. Run the Cetus checkpoint processor:

```bash
./run_cetus_processor.sh
```

Alternatively, you can run it directly:

```bash
cargo run --bin cetus_checkpoint_processor
```

### Output

The processor will output detailed information about Cetus transactions, including:

- Transaction digest
- Checkpoint sequence number
- Full transaction data

## Cetus Package ID

The indexer tracks transactions related to the Cetus package with ID:
`0xc6faf3703b0e8ba9ed06b7851134bbbe7565eb35ff823fd78432baa4cbeaa12e`

## How It Works

1. Checkpoint Processing: The indexer processes each checkpoint in sequence
2. Transaction Filtering: It identifies Cetus-related transactions by:
   - Checking if input/output objects are related to Cetus
   - Checking if transaction data references the Cetus package ID
3. Data Extraction: It extracts and logs relevant data from each matching transaction

## Customization

To customize the index for different Sui packages or objects:
- Modify the `CETUS_PACKAGE_ID` constant in `src/cetus_indexer.rs`
- Update the logic in `is_cetus_related()` and `is_cetus_transaction()` methods
# tasmil-custom-indexer
# tasmil-custom-indexer
# tasmil-custom-indexer
# custom-indexer-tasmil
# tasmil-custom-indexer
# tasmil-custom-indexer
# tasmil-indexer
