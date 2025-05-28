# SuiNS Indexer - Cetus DEX Data Processor

## 📋 Overview

The **SuiNS Indexer** is a high-performance Rust application that processes Sui blockchain checkpoints to extract and analyze data from the Cetus DEX (Decentralized Exchange). It monitors the SUI/USDC trading pool to calculate real-time metrics including:

- 📊 **24-hour Trading Volume** (in USD)
- 💎 **Total Value Locked (TVL)** (in USD) 
- 💰 **24-hour Trading Fees** (in USD)
- 🔄 **Real-time Swap Events**
- 💧 **Liquidity Provision Events**

## 🚀 Key Features

- **Real-time Processing**: Processes Sui blockchain checkpoints as they arrive
- **Thread-safe Architecture**: Concurrent checkpoint processing with shared state management
- **Database Persistence**: PostgreSQL storage for metrics and checkpoint progress
- **Price Integration**: Live price feeds from Pyth Network oracle
- **Configurable Settings**: Environment-based configuration management
- **Background Jobs**: Automatic metric updates every 10 minutes
- **Memory Efficient**: Automatic cleanup of old events to prevent memory growth
- **Comprehensive Logging**: Detailed event tracking and metric reporting

## 🏗️ Architecture

```
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   Sui Network   │────│  Checkpoint      │────│   Event         │
│   Checkpoints   │    │  Processor       │    │   Extractor     │
└─────────────────┘    └──────────────────┘    └─────────────────┘
                                │
                                ▼
┌─────────────────┐    ┌──────────────────┐    ┌─────────────────┐
│   Pyth Oracle   │────│  Metrics         │────│   Database      │
│  (Price Feeds)  │    │  Calculator      │    │  (PostgreSQL)   │
└─────────────────┘    └──────────────────┘    └─────────────────┘
```

### Key Components

1. **Checkpoint Processor** (`src/bin/cetus_checkpoint_processor.rs`)
   - Main entry point that coordinates all processing
   - Manages worker pools for concurrent checkpoint processing
   - Handles graceful shutdown and progress tracking

2. **Cetus Indexer** (`src/cetus_indexer.rs`)
   - Core business logic for extracting swap and liquidity events
   - Calculates 24-hour metrics (volume, TVL, fees)
   - Manages event storage and cleanup

3. **Configuration Management** (`src/config.rs`)
   - Environment variable loading and validation
   - Thread-safe global configuration access
   - Comprehensive validation of addresses and settings

4. **Database Layer** (`src/database.rs`)
   - PostgreSQL integration using Diesel ORM
   - Atomic operations for data consistency
   - Progress tracking and metric persistence

## 🛠️ Prerequisites

- **Rust** (1.70+ recommended)
- **PostgreSQL** (12+ recommended)
- **Internet Connection** (for Pyth price feeds and Sui network access)

## 📦 Installation

1. **Clone the repository:**
   ```bash
   git clone <repository-url>
   cd suins-indexer
   ```

2. **Install dependencies:**
   ```bash
   cargo build --release
   ```

3. **Set up PostgreSQL database:**
   ```bash
   # Create database
   createdb suins_indexer
   
   # Apply migrations (will be created automatically on first run)
   ```

## ⚙️ Configuration

### Environment Variables Setup

Create a `.env` file in the project root with the following variables:

```env
# ==============================================================================
# DATABASE CONFIGURATION
# ==============================================================================

# PostgreSQL connection string
# Format: postgresql://username:password@host:port/database_name
# Example for local development:
DATABASE_URL=postgresql://postgres:password@localhost:5432/suins_indexer
# Example for production:
# DATABASE_URL=postgresql://user:secure_password@db.example.com:5432/suins_indexer

# ==============================================================================
# CETUS DEX CONFIGURATION  
# ==============================================================================

# SUI/USDC Pool ID on Cetus DEX (66 characters with 0x prefix)
# This is the specific liquidity pool we're monitoring for trading activity
# Find this from Cetus DEX interface or their documentation
SUI_USDC_POOL_ID=0x4e2ca1213d8584b7e98dfd60aede3b2c6571dc25bde5838a9efe8f809bde8a73

# Cetus Protocol Contract Address (64 characters, no 0x prefix)  
# This is used to filter events to only those from Cetus protocol
# Example address - replace with actual Cetus address:
CETUS_ADDRESS=9e96e93cc7699b4e5d2b24ab4e9c3b5c6c43e0f8a9c6b2d9e4f6b8a1c3d5e7f9

# ==============================================================================
# PRICE FEED CONFIGURATION (Pyth Network)
# ==============================================================================

# USDC/USD Price Feed ID from Pyth Network (64 characters)
# Used to convert USDC amounts to USD for calculations
# Pyth Network provides real-time cryptocurrency price feeds
USDC_PRICE_FEED_ID=eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a

# SUI/USD Price Feed ID from Pyth Network (64 characters) 
# Critical for converting SUI amounts to USD for volume calculations
# This is the official SUI price feed from Pyth:
SUI_PRICE_FEED_ID=23d7315113f5b1d3ba7a83604c44b94d79f4fd69af77f804fc7f920a6dc65744

# Pyth Network API Endpoint (optional - has default)
# Default: https://hermes.pyth.network/api/latest_price_feeds
# You can use this default or specify your own Pyth endpoint
PYTH_API_URL=https://hermes.pyth.network/api/latest_price_feeds

# ==============================================================================
# OPERATIONAL SETTINGS
# ==============================================================================

# Update interval for metrics calculation (in seconds)
# How often the background job updates volume/TVL metrics and saves to database
# Default: 600 (10 minutes) - minimum recommended: 60 seconds
UPDATE_INTERVAL_SECONDS=600

# Event retention period (in hours)
# How long to keep events in memory for 24h calculations
# Default: 25 (gives buffer over 24h) - minimum: 1 hour
EVENT_RETENTION_HOURS=25

# Checkpoint batch size
# How many checkpoints to process before forcing a database update
# Default: 10 - range: 1-100
CHECKPOINT_BATCH_SIZE=10

# ==============================================================================
# SUI NETWORK CONFIGURATION (Optional)
# ==============================================================================

# Remote storage URL for downloading checkpoints
# If not provided, assumes checkpoints are available locally
# Example: https://checkpoints.sui.io
# REMOTE_STORAGE=https://checkpoints.mainnet.sui.io

# Path to backfill progress tracking file
# Used to resume processing from the last checkpoint after restarts
# Default: ./backfill_progress/backfill_progress
BACKFILL_PROGRESS_FILE_PATH=./backfill_progress/backfill_progress

# ==============================================================================
# RUNTIME CONTROL
# ==============================================================================

# Enable/disable database functionality
# Set to "true" for production, "false" for testing without database
USE_DATABASE=true

# Directory where checkpoints are stored/downloaded
# Default: ./checkpoints
CHECKPOINTS_DIR=./checkpoints
```

### Configuration Validation

The application performs comprehensive validation of all configuration values:

- **Database URL**: Must be a valid PostgreSQL connection string
- **Pool ID**: Must be 66 characters (0x + 64 hex characters)
- **Addresses**: Must be valid 32-byte hex strings
- **Price Feed IDs**: Must be valid Pyth Network feed identifiers
- **Intervals**: Must be reasonable values (min 60 seconds)
- **URLs**: Must be valid HTTP/HTTPS endpoints

## 🚀 Usage

### Running the Indexer

1. **Start the checkpoint processor:**
   ```bash
   cargo run --bin cetus_checkpoint_processor
   ```

2. **Using the shell script (recommended):**
   ```bash
   chmod +x run_cetus_processor.sh
   ./run_cetus_processor.sh
   ```

### Expected Output

When running successfully, you'll see output like:

```
🚀 Starting SUI-USDC Pool Volume & TVL Indexer
📁 Checkpoints dir: ./checkpoints
💾 Database enabled: true
🎯 Target pool: 0x4e2ca121...8a73

📋 Configuration Summary:
  🏊 Pool ID: 0x4e2ca121...8a73
  🏢 Cetus Address: 9e96e93c...e7f9
  ⏱️  Update Interval: 600s
  🗂️  Event Retention: 25h
  📦 Batch Size: 10
  🌐 Pyth API: https://hermes.pyth.network/api/latest_price_feeds

✅ Loaded last processed checkpoint: 147833580
✅ Loaded data from database:
   24h Volume: $2.45M
   24h TVL: $8.32M  
   24h Fees: $7.35K

⏳ Starting checkpoint processing...

Processing checkpoint 147833581 with 234 transactions
Found 3 Cetus swap events
SWAP EVENT #1
Transaction: 0x1a2b3c4d...
Pool ID: 0x4e2ca121...8a73
Amount In: 1000000
Amount Out: 534223
Fee Amount: 3000
Direction: USDC -> SUI
SUI Amount: 534223

📊 Checkpoint 147833581: 3 swap events, 1 liquidity events | Volume: $2.47 | TVL: $8.35 | Fees: $7.37
```

## 📊 Data Flow

### 1. Checkpoint Processing
```
Sui Network → Checkpoint Data → Transaction Analysis → Event Extraction
```

### 2. Event Processing  
```
Raw Events → Validation → Parsing → Storage → Metric Calculation
```

### 3. Metric Calculation
```
Swap Events → Volume Calculation → USD Conversion → Database Storage
Liquidity Events → TVL Calculation → USD Conversion → Database Storage
Fee Events → Fee Calculation → USD Conversion → Database Storage
```

### 4. Data Persistence
```
Real-time Updates → Database Upsert → Progress Tracking → Background Jobs
```

## 🗃️ Database Schema

The application creates and manages the following database structure:

```sql
-- Main metrics table
CREATE TABLE volume_data (
    id SERIAL PRIMARY KEY,
    period VARCHAR NOT NULL DEFAULT '24h',
    sui_usd_volume NUMERIC NOT NULL DEFAULT 0,
    total_usd_tvl NUMERIC NOT NULL DEFAULT 0,
    fees_24h NUMERIC NOT NULL DEFAULT 0,
    last_update TIMESTAMP NOT NULL DEFAULT NOW(),
    last_processed_checkpoint BIGINT NOT NULL DEFAULT 0
);

-- Indexes for performance
CREATE INDEX idx_volume_data_period ON volume_data(period);
CREATE INDEX idx_volume_data_last_update ON volume_data(last_update);
CREATE INDEX idx_volume_data_checkpoint ON volume_data(last_processed_checkpoint);
```

## 🔧 Development

### Running Tests
```bash
cargo test
```

### Code Structure
```
src/
├── bin/
│   └── cetus_checkpoint_processor.rs  # Main application entry point
├── cetus_indexer.rs                   # Core indexing logic
├── config.rs                          # Configuration management  
├── database.rs                        # Database operations
├── lib.rs                            # Library exports
└── schema.rs                         # Database schema (auto-generated)
```

### Adding New Features

1. **New Event Types**: Extend the `extract_events()` function in `cetus_indexer.rs`
2. **Additional Metrics**: Add calculation functions following the pattern in `calculate_volume_24h()`
3. **Database Fields**: Update schema and add migration files
4. **Configuration**: Add new fields to `Config` struct in `config.rs`

## 🚨 Troubleshooting

### Common Issues

1. **Database Connection Errors**
   ```
   Error: Failed to connect to database
   Solution: Check DATABASE_URL and ensure PostgreSQL is running
   ```

2. **Configuration Validation Errors**
   ```
   Error: SUI_USDC_POOL_ID must be a valid 32-byte hex string
   Solution: Verify the pool ID format (66 chars with 0x prefix)
   ```

3. **Price Feed Errors**
   ```
   Error: Failed to get SUI price, using fallback
   Solution: Check internet connection and Pyth Network availability
   ```

4. **Checkpoint Processing Stalls**
   ```
   Issue: No new checkpoints being processed
   Solution: Check REMOTE_STORAGE setting and network connectivity
   ```

### Debugging

Enable detailed logging:
```bash
RUST_LOG=debug cargo run --bin cetus_checkpoint_processor
```

Check database connection:
```bash
psql $DATABASE_URL -c "SELECT version();"
```

## 🔒 Security Considerations

- **Environment Variables**: Store sensitive configuration in `.env` files, not in code
- **Database Security**: Use strong passwords and connection encryption in production
- **API Keys**: If using custom Pyth endpoints, secure API keys appropriately
- **Network Security**: Ensure secure connections to external APIs

## 📈 Performance Optimization

### Memory Management
- Events older than 24 hours are automatically pruned
- Configurable retention periods to balance accuracy vs memory usage

### Database Performance  
- Indexed queries for fast metric retrieval
- Batch updates to reduce database load
- Connection pooling for concurrent access

### Network Efficiency
- Efficient checkpoint processing with worker pools
- Minimal API calls to price feeds
- Graceful handling of network failures

## 🤝 Contributing

1. Fork the repository
2. Create a feature branch
3. Add comprehensive tests
4. Update documentation
5. Submit a pull request

## 📄 License

This project is licensed under the Apache 2.0 License - see the LICENSE file for details.

## 🆘 Support

For issues and questions:
1. Check the troubleshooting section above
2. Review the logs for detailed error messages  
3. Ensure all environment variables are correctly set
4. Verify database connectivity and migrations

---

**Note**: This indexer is designed for the Sui blockchain and Cetus DEX specifically. Adapting it for other blockchains or DEXs would require significant modifications to the event parsing and metric calculation logic.
