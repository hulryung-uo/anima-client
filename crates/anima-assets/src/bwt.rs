//! Burrows–Wheeler (BWT) decompressor used by newer cliloc / gump / anim
//! payloads. Port of ClassicUO `ClassicUO.Utility.BwtDecompress`.
//!
//! A buffer whose 4th byte is `0x8E` is BWT-compressed (ClassicUO
//! `ClilocLoader.ReadCliloc`). Uncompressed tables skip this entirely.

/// Decompress a BWT-framed buffer. Returns an empty vec when the header's
/// frequency table does not sum to a coherent length — ClassicUO does the
/// same rather than panicking on a corrupt file.
pub fn decompress(buffer: &[u8]) -> Vec<u8> {
    if buffer.len() < 5 {
        return Vec::new();
    }
    let mut first_char = buffer[4];
    let mut table = [0u16; 256 * 256];
    build_table(&mut table, first_char);

    let mut list = vec![0u8; buffer.len().saturating_sub(4)];
    let mut i = 0usize;
    let mut pos = 5usize;
    while pos < buffer.len() {
        let mut current = first_char as usize;
        let value = table[current];
        while current > 0 {
            table[current] = table[current - 1];
            current -= 1;
        }
        table[0] = value;
        if i < list.len() {
            list[i] = value as u8;
            i += 1;
        }
        first_char = buffer[pos];
        pos += 1;
    }
    list.truncate(i);
    internal_decompress(&list)
}

fn build_table(table: &mut [u16], start_value: u8) {
    let mut first = start_value;
    let mut second = 0u8;
    for slot in table.iter_mut() {
        *slot = u16::from(first) + (u16::from(second) << 8);
        first = first.wrapping_add(1);
        if first == 0 {
            second = second.wrapping_add(1);
        }
    }
    table.sort_unstable();
}

fn internal_decompress(input: &[u8]) -> Vec<u8> {
    if input.len() < 1024 {
        return Vec::new();
    }
    let mut symbol_table = [0u8; 256];
    let mut frequency = [0u8; 256];
    let mut partial = [0i32; 256 * 3];
    for i in 0..256 {
        symbol_table[i] = i as u8;
    }
    // First 1024 bytes are 256 little-endian i32 frequency counts.
    for i in 0..256 {
        let o = i * 4;
        partial[i] = i32::from_le_bytes([input[o], input[o + 1], input[o + 2], input[o + 3]]);
    }
    let sum: i32 = partial[..256].iter().sum();
    if sum <= 0 {
        return Vec::new();
    }
    let len = sum as usize;
    let mut output = vec![0u8; len];

    let mut non_zero = 0i32;
    for i in 0..256 {
        if partial[i] != 0 {
            non_zero += 1;
        }
    }
    fill_frequency(&partial[..256], &mut frequency);

    let mut m = 0i32;
    for i in 0..non_zero {
        let freq = frequency[i as usize];
        if 1024 + m as usize >= input.len() {
            return Vec::new();
        }
        symbol_table[input[1024 + m as usize] as usize] = freq;
        partial[freq as usize + 256] = m + 1;
        m += partial[freq as usize];
        partial[freq as usize + 512] = m;
    }

    let mut val = symbol_table[0];
    let mut count = 0usize;
    let mut remaining_nonzero = non_zero;
    while count < len {
        let first_idx = val as usize + 256;
        output[count] = val;
        if partial[first_idx] >= partial[val as usize + 512] {
            remaining_nonzero -= 1;
            if remaining_nonzero >= 0 {
                shift_left(&mut symbol_table, remaining_nonzero as usize);
                val = symbol_table[0];
            }
        } else {
            let idx_off = 1024 + partial[first_idx] as usize;
            if idx_off >= input.len() {
                break;
            }
            let idx = input[idx_off];
            partial[first_idx] += 1;
            if idx != 0 {
                shift_left(&mut symbol_table, idx as usize);
                symbol_table[idx as usize] = val;
                val = symbol_table[0];
            }
        }
        count += 1;
    }
    output
}

fn fill_frequency(input: &[i32], output: &mut [u8]) {
    let mut tmp = [0i32; 256];
    tmp.copy_from_slice(input);
    for slot in output.iter_mut() {
        let mut value = 0u32;
        let mut index = 0u8;
        for (j, &n) in tmp.iter().enumerate() {
            if n > 0 && (n as u32) > value {
                index = j as u8;
                value = n as u32;
            }
        }
        if value == 0 {
            break;
        }
        *slot = index;
        tmp[index as usize] = 0;
    }
}

fn shift_left(input: &mut [u8], max: usize) {
    for i in 0..max {
        if i + 1 < input.len() {
            input[i] = input[i + 1];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_short_buffers_are_inert() {
        assert!(decompress(&[]).is_empty());
        assert!(decompress(&[1, 2, 3, 0x8E]).is_empty());
    }

    #[test]
    fn build_table_is_sorted_pairs() {
        let mut table = [0u16; 256 * 256];
        build_table(&mut table, 0);
        for w in table.windows(2) {
            assert!(w[0] <= w[1]);
        }
        // Every (lo, hi) pair appears once.
        assert_eq!(table[0], 0);
        assert_eq!(table[table.len() - 1], 0xFFFF);
    }
}
