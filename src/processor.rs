use memmap2::Mmap;
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct CsvProcessor {
    delimiter: u8,
}

impl CsvProcessor {
    pub fn new(delimiter: char) -> Self {
        Self {
            delimiter: delimiter as u8,
        }
    }

    pub fn process_file(&self, input_path: &Path, output_path: &Path) -> Result<usize, Box<dyn std::error::Error>> {
        let file = File::open(input_path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        
        // Find line boundaries for parallel processing
        let chunk_size = mmap.len() / rayon::current_num_threads().max(1);
        let mut chunk_boundaries = vec![0];
        
        let mut pos = 0;
        while pos < mmap.len() {
            let end = (pos + chunk_size).min(mmap.len());
            let boundary = self.find_line_boundary(&mmap, end);
            chunk_boundaries.push(boundary);
            pos = boundary;
        }
        
        if chunk_boundaries.last() != Some(&mmap.len()) {
            chunk_boundaries.push(mmap.len());
        }
        
        let processed_count = AtomicUsize::new(0);
        
        // Process chunks in parallel
        let results: Vec<_> = chunk_boundaries
            .windows(2)
            .map(|bounds| {
                let start = bounds[0];
                let end = bounds[1];
                &mmap[start..end]
            })
            .collect();
            
        let output_data: Vec<_> = results
            .into_par_iter()
            .map(|chunk| self.process_chunk(chunk))
            .collect();
        
        // Write results sequentially to maintain order
        let mut writer = BufWriter::new(File::create(output_path)?);
        for data in output_data {
            if !data.is_empty() {
                writer.write_all(&data)?;
                processed_count.fetch_add(
                    data.iter().filter(|&&b| b == b'\n').count(),
                    Ordering::Relaxed
                );
            }
        }
        
        Ok(processed_count.into_inner())
    }

    pub fn process_file_with_filter(
        &self,
        input_path: &Path,
        output_path: &Path,
        progress_counter: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
        fields_to_extract: &[usize],
        filter_equal: Option<(usize, usize)>,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let file = File::open(input_path)?;
        let mmap = unsafe { Mmap::map(&file)? };
        
        // Find line boundaries for parallel processing
        let chunk_size = mmap.len() / rayon::current_num_threads().max(1);
        let mut chunk_boundaries = vec![0];
        
        let mut pos = 0;
        while pos < mmap.len() {
            let end = (pos + chunk_size).min(mmap.len());
            let boundary = self.find_line_boundary(&mmap, end);
            chunk_boundaries.push(boundary);
            pos = boundary;
        }
        
        if chunk_boundaries.last() != Some(&mmap.len()) {
            chunk_boundaries.push(mmap.len());
        }
        
        // Process chunks in parallel with filtering
        let results: Vec<_> = chunk_boundaries
            .windows(2)
            .map(|bounds| {
                let start = bounds[0];
                let end = bounds[1];
                &mmap[start..end]
            })
            .collect();
            
        let output_data: Vec<_> = results
            .into_par_iter()
            .map(|chunk| {
                let result = self.process_chunk_with_filter(chunk, fields_to_extract, filter_equal);
                progress_counter.fetch_add(
                    result.iter().filter(|&&b| b == b'\n').count(),
                    Ordering::Relaxed
                );
                result
            })
            .collect();
        
        // Write results sequentially to maintain order
        let mut writer = BufWriter::new(File::create(output_path)?);
        for data in output_data {
            if !data.is_empty() {
                writer.write_all(&data)?;
            }
        }
        
        Ok(progress_counter.load(Ordering::Relaxed))
    }

    fn process_chunk_with_filter(
        &self,
        chunk: &[u8],
        fields_to_extract: &[usize],
        filter_equal: Option<(usize, usize)>,
    ) -> Vec<u8> {
        let mut result = Vec::new();
        let mut lines = chunk.split(|&b| b == b'\n');
        
        // Skip header if this is the first chunk
        let skip_header = chunk == &chunk[0..];
        if skip_header {
            let _ = lines.next();
        }
        
        for line in lines {
            if line.is_empty() {
                continue;
            }
            
            if let Some(extracted) = self.extract_and_filter(line, fields_to_extract, filter_equal) {
                result.extend_from_slice(&extracted);
                result.push(b'\n');
            }
        }
        
        result
    }

    fn extract_and_filter(
        &self,
        line: &[u8],
        fields_to_extract: &[usize],
        filter_equal: Option<(usize, usize)>,
    ) -> Option<Vec<u8>> {
        let fields: Vec<&[u8]> = line.split(|&b| b == self.delimiter).collect();
        
        // Skip if we don't have enough fields
        if fields.len() <= fields_to_extract.iter().max().copied().unwrap_or(0) {
            return None;
        }
        
        // Apply filter if specified
        if let Some((col1, col2)) = filter_equal {
            if col1 < fields.len() && col2 < fields.len() && fields[col1] == fields[col2] {
                return None;
            }
        }
        
        // Extract requested fields
        let mut result = Vec::new();
        for (i, &field_idx) in fields_to_extract.iter().enumerate() {
            if field_idx < fields.len() {
                if !fields[field_idx].is_empty() {
                    if i > 0 {
                        result.push(b',');
                    }
                    result.extend_from_slice(fields[field_idx]);
                }
            }
        }
        
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }
    
    fn find_line_boundary(&self, data: &[u8], mut pos: usize) -> usize {
        while pos < data.len() && data[pos] != b'\n' {
            pos += 1;
        }
        if pos < data.len() {
            pos + 1
        } else {
            data.len()
        }
    }
    
    fn process_chunk(&self, chunk: &[u8]) -> Vec<u8> {
        let mut result = Vec::new();
        let mut lines = chunk.split(|&b| b == b'\n');
        
        // Skip header if this is the first chunk
        let skip_header = chunk == &chunk[0..];
        if skip_header {
            let _ = lines.next();
        }
        
        for line in lines {
            if line.is_empty() {
                continue;
            }
            
            if let Some((email, username)) = self.extract_columns(line) {
                result.extend_from_slice(email);
                result.push(b',');
                result.extend_from_slice(username);
                result.push(b'\n');
            }
        }
        
        result
    }
    
    fn extract_columns<'a>(&self, line: &'a [u8]) -> Option<(&'a [u8], &'a [u8])> {
        let mut fields = line.split(|&b| b == self.delimiter);
        
        // Skip user_id
        let _ = fields.next()?;
        
        // Get email (column 2)
        let email = fields.next()?;
        
        // Get username_or_id (column 3)
        let username = fields.next()?;
        
        Some((email, username))
    }
}