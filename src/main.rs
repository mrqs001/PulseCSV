use clap::Parser;
use std::path::PathBuf;
use std::time::Instant;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::io::{self, Write};

mod processor;

use processor::CsvProcessor;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input CSV file path
    #[arg(short, long)]
    input: PathBuf,

    /// Output file path
    #[arg(short, long)]
    output: PathBuf,

    /// Field delimiter
    #[arg(short, long, default_value = ":")]
    delimiter: char,

    /// Number of threads to use
    #[arg(short, long)]
    threads: Option<usize>,

    /// Fields to extract (comma-separated indices, 0-based)
    #[arg(short, long, default_value = "1,2")]
    fields: String,

    /// Filter rows where two columns are equal (format: col1,col2)
    #[arg(long)]
    filter_equal: Option<String>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
    // Set thread count if specified
    if let Some(threads) = args.threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build_global()
            .unwrap();
    }

    let start = Instant::now();
    
    // Parse field indices
    let fields_to_extract: Vec<usize> = args.fields
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();
    
    // Parse filter columns if provided
    let filter_equal = args.filter_equal.as_ref().map(|s| {
        let parts: Vec<usize> = s.split(',').map(|s| s.trim().parse().unwrap()).collect();
        (parts[0], parts[1])
    });
    
    // Start progress reporting thread
    let file_size = args.input.metadata()?.len();
    let progress_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter_clone = progress_counter.clone();
    
    let _progress_thread = thread::spawn(move || {
        let mut last_lines = 0;
        loop {
            thread::sleep(Duration::from_millis(100));
            let current_lines = counter_clone.load(Ordering::Relaxed);
            if current_lines > last_lines {
                let elapsed = start.elapsed();
                let mb_processed = (current_lines as f64 * 50.0) / (1024.0 * 1024.0);
                let throughput = mb_processed / elapsed.as_secs_f64();
                
                // Clear line and show simple progress
                print!("\rProcessing: {} lines | {:.1} MB/s", current_lines, throughput);
                io::stdout().flush().unwrap();
                last_lines = current_lines;
            }
        }
    });
    
    let processor = CsvProcessor::new(args.delimiter);
    let processed_lines = processor.process_file_with_filter(
        &args.input,
        &args.output,
        &progress_counter,
        &fields_to_extract,
        filter_equal
    )?;
    
    let duration = start.elapsed();
    
    // Clear the progress line and show completion
    print!("\r");
    println!("✅ Complete! {} lines processed in {:.1}s", processed_lines, duration.as_secs_f64());
    println!("📊 Speed: {:.1} MB/s", (file_size as f64 / 1024.0 / 1024.0) / duration.as_secs_f64());
    
    Ok(())
}