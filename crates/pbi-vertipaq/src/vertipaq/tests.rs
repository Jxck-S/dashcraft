//! Unit tests on synthetic `.idfmeta` / `.idf` / `.dictionary` buffers and a synthetic
//! ABF backup (no customer data).

use arrow::array::{
    Array, AsArray, BinaryArray, BooleanArray, Decimal128Array, Float64Array, Int64Array,
    StringArray, TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType as ArrowType, TimeUnit};
use pbi_model::DataType;
use rusqlite::Connection;

use super::convert::{self, Source, base64_decode, days_to_micros};
use super::huffman::{BitStream, HuffmanTable};
use super::idfmeta::{
    FIRST_DATA_ID, NO_SPLIT_CLASS_BASE, NULL_DATA_ID, XM123_CLASS, no_split_bit_width,
};
use super::test_support::*;
use super::*;
use crate::abf::AbfArchive;
use crate::abf::test_support::AbfBuilder;
use crate::metadata::{ModelOptions, load_model};

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn decode(meta: &[u8], idf: &[u8]) -> Result<Vec<u32>, VertipaqError> {
    let meta = parse_idfmeta(meta)?;
    let mut ids = Vec::new();
    decode_idf(idf, &meta, u64::MAX, &mut ids)?;
    Ok(ids)
}

fn corrupt_message(err: &VertipaqError) -> String {
    err.to_string()
}

// ---------------------------------------------------------------- idfmeta

#[test]
fn idfmeta_parses_segments_and_sdos() {
    let mut nulls = SegSpec::hybrid(486, 6, 3, true);
    nulls.sub_records = Some(21);
    let bytes = idfmeta_bytes(&[nulls, SegSpec::packed(10, 12, 7)], true);
    let meta = parse_idfmeta(&bytes).unwrap();
    assert_eq!(meta.segments.len(), 2);
    assert_eq!(meta.records(), Some(496));
    let first = &meta.segments[0];
    assert_eq!(
        first.encoding(),
        SegmentEncoding::HybridRle { bit_width: 6 }
    );
    assert_eq!(first.subsegment.unwrap().records, 21);
    assert!(first.stats.has_nulls);
    assert_eq!(first.stats.min_data_id, 3);
    assert_eq!(first.bit_pack_base(), NULL_DATA_ID);
    assert_eq!(first.rle.unwrap().storage_alloc_size, 16);
    let second = &meta.segments[1];
    assert_eq!(
        second.encoding(),
        SegmentEncoding::BitPacked { bit_width: 12 }
    );
    assert_eq!(second.bit_pack_base(), 7);
    assert!(second.rle.is_none() && second.subsegment.is_none());
    let objects = meta.data_objects.unwrap();
    assert_eq!(objects[0].subsegment, Some((8, 16)));
    assert_eq!(objects[1].subsegment, None);

    // Without SDOs, and with unknown trailing bytes.
    let mut plain = idfmeta_bytes(&[SegSpec::hybrid(1, 1, 3, false)], false);
    assert!(parse_idfmeta(&plain).unwrap().data_objects.is_none());
    plain.extend_from_slice(b"junk");
    assert_eq!(parse_idfmeta(&plain).unwrap().segments.len(), 1);
}

#[test]
fn compression_classes_map_to_bit_widths() {
    for (class, width) in [
        (0xABA37, Some(1)),
        (0xABA40, Some(10)),
        (0xABA42, Some(12)),
        (0xABA46, Some(16)),
        (0xABA4B, Some(21)),
        (0xABA56, Some(32)),
        (0xABA36, None),
        (0xABA57, None),
        (0, None),
    ] {
        assert_eq!(no_split_bit_width(class), width, "{class:#x}");
    }
    let mut spec = SegSpec::hybrid(1, 1, 3, false);
    spec.sub_class = XM123_CLASS;
    let meta = parse_idfmeta(&idfmeta_bytes(&[spec], false)).unwrap();
    assert!(matches!(
        meta.segments[0].encoding(),
        SegmentEncoding::Unsupported { .. }
    ));
}

#[test]
fn idfmeta_rejects_corrupt_input() {
    let good = idfmeta_bytes(&[SegSpec::hybrid(5, 3, 3, false)], true);
    // Every truncation fails cleanly (the SDOs block is optional, so cutting exactly
    // before it is valid).
    let cp_end = good.windows(6).position(|w| w == b"CP:1>\0").unwrap() + 6;
    for len in 0..good.len() {
        let result = parse_idfmeta(&good[..len]);
        if len == cp_end {
            assert!(result.is_ok());
        } else if len < cp_end {
            assert!(result.is_err(), "prefix {len}");
        }
    }
    let mut bad_tag = good.clone();
    bad_tag[1] = b'2';
    assert!(matches!(
        parse_idfmeta(&bad_tag),
        Err(VertipaqError::Corrupt { .. })
    ));

    let mut huge = good.clone();
    huge[6..14].copy_from_slice(&u64::MAX.to_le_bytes());
    let err = parse_idfmeta(&huge).unwrap_err();
    assert!(corrupt_message(&err).contains("segments declared"), "{err}");

    // A subsegment that declares its own subsegment.
    let sub_flag = good.windows(7).position(|w| w == b"\x01<1:CS\0").unwrap();
    let nested_flag = sub_flag + 7 + 16;
    let mut nested = good.clone();
    nested[nested_flag] = 1;
    let err = parse_idfmeta(&nested).unwrap_err();
    assert!(corrupt_message(&err).contains("nested"), "{err}");
}

// ---------------------------------------------------------------- idf

#[test]
fn hybrid_round_trip_for_every_bit_width() {
    let mut rng = Rng(0x5EED);
    for width in 1..=32u8 {
        let base = FIRST_DATA_ID;
        let span = if width == 32 {
            u64::from(u32::MAX - base)
        } else {
            (1u64 << width) - 1
        };
        let mut ids = Vec::new();
        while ids.len() < 700 {
            let id = base + rng.below(span + 1) as u32;
            let run = if rng.below(5) == 0 {
                64 + rng.below(100)
            } else {
                1
            };
            ids.extend(std::iter::repeat_n(id, run as usize));
        }
        let meta = idfmeta_bytes(
            &[SegSpec::hybrid(ids.len() as u64, width, base, false)],
            false,
        );
        let idf = encode_hybrid(&ids, base, width, 64, 3);
        assert_eq!(decode(&meta, &idf).unwrap(), ids, "width {width}");

        let packed_meta = idfmeta_bytes(&[SegSpec::packed(ids.len() as u64, width, base)], false);
        let offsets: Vec<u32> = ids.iter().map(|id| id - base).collect();
        let words = pack(&offsets, width);
        let mut packed_idf = (words.len() as u64).to_le_bytes().to_vec();
        for w in &words {
            packed_idf.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(
            decode(&packed_meta, &packed_idf).unwrap(),
            ids,
            "plain width {width}"
        );
    }
}

#[test]
fn bit_packing_sentinel_slides_with_the_offset() {
    // [packed 3] [run of 7 × 64] [packed 2]: the second packing entry is -1 - 3.
    let words = pack(&[0, 1, 2, 3, 4], 3);
    let idf = hybrid_segment(&[(u32::MAX, 3), (7, 64), (u32::MAX - 3, 2), (0, 0)], &words);
    let meta = idfmeta_bytes(&[SegSpec::hybrid(69, 3, 10, false)], false);
    let ids = decode(&meta, &idf).unwrap();
    let mut expected = vec![10, 11, 12];
    expected.extend(std::iter::repeat_n(7, 64));
    expected.extend([13, 14]);
    assert_eq!(ids, expected);

    // A packing entry with the wrong offset is an ordinary RLE run.
    let idf = hybrid_segment(&[(u32::MAX, 3), (u32::MAX, 2)], &words);
    let meta = idfmeta_bytes(&[SegSpec::hybrid(5, 3, 10, false)], false);
    assert_eq!(
        decode(&meta, &idf).unwrap(),
        vec![10, 11, 12, u32::MAX, u32::MAX]
    );
}

#[test]
fn stale_padding_is_capped_at_records() {
    let idf = hybrid_segment(&[(5, 100), (9, u32::MAX), (u32::MAX - 100, 7)], &[0]);
    let meta = idfmeta_bytes(&[SegSpec::hybrid(100, 1, 5, false)], false);
    assert_eq!(decode(&meta, &idf).unwrap(), vec![5; 100]);

    // A run larger than the segment is cut at the record count.
    let meta = idfmeta_bytes(&[SegSpec::hybrid(40, 1, 5, false)], false);
    assert_eq!(decode(&meta, &idf).unwrap(), vec![5; 40]);
}

#[test]
fn null_segments_pack_relative_to_the_null_id() {
    // Values 2 (NULL) and 3..=5 packed with base 2 even though min_data_id is 3.
    let ids = vec![2, 3, 5, 2, 4];
    let idf = encode_hybrid(&ids, NULL_DATA_ID, 2, 64, 0);
    let meta = idfmeta_bytes(&[SegSpec::hybrid(5, 2, 3, true)], false);
    assert_eq!(decode(&meta, &idf).unwrap(), ids);
}

#[test]
fn single_zero_word_means_every_value_is_the_base() {
    let idf = hybrid_segment(&[(u32::MAX, 200)], &[0]);
    let meta = idfmeta_bytes(&[SegSpec::hybrid(200, 1, 3, false)], false);
    assert_eq!(decode(&meta, &idf).unwrap(), vec![3; 200]);
    // With a non-zero word the same layout is truncated.
    let idf = hybrid_segment(&[(u32::MAX, 200)], &[1]);
    let err = decode(&meta, &idf).unwrap_err();
    assert!(corrupt_message(&err).contains("beyond"), "{err}");
}

#[test]
fn several_segments_and_empty_segments() {
    let first = encode_hybrid(&[3, 4, 3], 3, 1, 64, 0);
    let empty = hybrid_segment(&[(0, 0); 16], &[0]);
    let second = encode_hybrid(&[6; 70], 6, 1, 64, 1);
    let idf = [first, empty, second].concat();
    let meta = idfmeta_bytes(
        &[
            SegSpec::hybrid(3, 1, 3, false),
            SegSpec::hybrid(0, 1, 2, false),
            SegSpec::hybrid(70, 1, 6, false),
        ],
        false,
    );
    let mut expected = vec![3, 4, 3];
    expected.extend([6; 70]);
    assert_eq!(decode(&meta, &idf).unwrap(), expected);

    // The last hybrid segment may omit its subsegment when the metadata has none.
    let mut spec = SegSpec::hybrid(4, 1, 8, false);
    spec.sub_records = None;
    let meta = idfmeta_bytes(&[spec], false);
    let mut idf = 1u64.to_le_bytes().to_vec();
    idf.extend_from_slice(&8u32.to_le_bytes());
    idf.extend_from_slice(&4u32.to_le_bytes());
    assert_eq!(decode(&meta, &idf).unwrap(), vec![8; 4]);
}

#[test]
fn idf_rejects_corrupt_input() {
    let ids: Vec<u32> = (0..50).map(|i| 3 + i % 7).collect();
    let meta = idfmeta_bytes(&[SegSpec::hybrid(50, 3, 3, false)], false);
    let idf = encode_hybrid(&ids, 3, 3, 64, 2);
    assert_eq!(decode(&meta, &idf).unwrap(), ids);
    for len in 0..idf.len() - 1 {
        assert!(decode(&meta, &idf[..len]).is_err(), "prefix {len}");
    }

    // Runs that do not cover the declared rows.
    let short = hybrid_segment(&[(3, 10)], &[0, 0]);
    let meta = idfmeta_bytes(&[SegSpec::hybrid(11, 1, 3, false)], false);
    let err = decode(&meta, &short).unwrap_err();
    assert!(corrupt_message(&err).contains("cover 10"), "{err}");

    // Bit widths outside 1..=32 and unknown classes.
    for class in [
        NO_SPLIT_CLASS_BASE,
        NO_SPLIT_CLASS_BASE + 33,
        XM123_CLASS,
        0,
    ] {
        let mut spec = SegSpec::hybrid(1, 1, 3, false);
        spec.sub_class = class;
        let meta = idfmeta_bytes(&[spec], false);
        let err = decode(&meta, &hybrid_segment(&[(3, 1)], &[0])).unwrap_err();
        assert!(
            matches!(err, VertipaqError::Unsupported { .. }),
            "{class:#x}: {err}"
        );
    }
    for width in [0u8, 33, 64, 255] {
        let err = idf::Unpacker::new(&[], width, 3).unwrap_err();
        assert!(corrupt_message(&err).contains("bit width"), "{err}");
    }

    // Data ids that overflow 32 bits.
    let mut spec = SegSpec::hybrid(1, 2, u32::MAX - 1, false);
    spec.sub_records = Some(1);
    let meta = idfmeta_bytes(&[spec], false);
    let err = decode(&meta, &hybrid_segment(&[(u32::MAX, 1)], &[3])).unwrap_err();
    assert!(corrupt_message(&err).contains("32 bits"), "{err}");

    // Huge declared sizes.
    let mut huge = hybrid_segment(&[(3, 1)], &[0]);
    huge[..8].copy_from_slice(&(u64::MAX / 4).to_le_bytes());
    let meta = idfmeta_bytes(&[SegSpec::hybrid(1, 1, 3, false)], false);
    assert!(matches!(
        decode(&meta, &huge),
        Err(VertipaqError::Truncated { .. })
    ));

    // Row limit.
    let meta = parse_idfmeta(&idfmeta_bytes(
        &[SegSpec::hybrid(1 << 40, 1, 3, false)],
        false,
    ))
    .unwrap();
    let mut out = Vec::new();
    let err = decode_idf(
        &hybrid_segment(&[(3, u32::MAX)], &[0]),
        &meta,
        1 << 20,
        &mut out,
    )
    .unwrap_err();
    assert!(matches!(err, VertipaqError::LimitExceeded { .. }), "{err}");
}

// ---------------------------------------------------------------- dictionary

#[test]
fn numeric_dictionaries() {
    let limits = DecodeLimits::default();
    assert_eq!(
        parse_dictionary(&long_dictionary(&[1, -2, 2_000_000_000], 4), &limits).unwrap(),
        Dictionary::Long(vec![1, -2, 2_000_000_000])
    );
    assert_eq!(
        parse_dictionary(&long_dictionary(&[i64::MIN, 7], 8), &limits).unwrap(),
        Dictionary::Long(vec![i64::MIN, 7])
    );
    let mut padded = real_dictionary(&[45000.5, -0.25]);
    padded.extend_from_slice(&[0; 16]);
    assert_eq!(
        parse_dictionary(&padded, &limits).unwrap(),
        Dictionary::Real(vec![45000.5, -0.25])
    );
}

#[test]
fn string_dictionary_plain_and_huffman_pages() {
    let pages = vec![
        PageSpec::Plain(strings(&["", "Alpha", "βeta", "emoji 😀"])),
        PageSpec::Huffman(
            strings(&["café", "Zürich", "", "aaaaab"]),
            Charset::Single(0),
        ),
        PageSpec::Huffman(strings(&["Привет", "мир"]), Charset::Single(0x04)),
        PageSpec::Huffman(strings(&["日本語😀", "mixed ascii"]), Charset::Multi),
        PageSpec::Plain(strings(&["tail"])),
    ];
    let bytes = string_dictionary(&pages);
    let Dictionary::String(values) = parse_dictionary(&bytes, &DecodeLimits::default()).unwrap()
    else {
        panic!("not a string dictionary");
    };
    let expected: Vec<String> = pages
        .iter()
        .flat_map(|p| match p {
            PageSpec::Plain(s) | PageSpec::Huffman(s, _) => s.clone(),
        })
        .collect();
    assert_eq!(values, expected);
}

#[test]
fn huffman_pages_with_odd_buffers_and_one_symbol() {
    // One symbol only: an incomplete (single-codeword) code.
    let one = string_dictionary(&[PageSpec::Huffman(
        strings(&["aaaa", "a", "aaaaaaaaaaaaaaaaa"]),
        Charset::Single(0),
    )]);
    assert_eq!(
        parse_dictionary(&one, &DecodeLimits::default()).unwrap(),
        Dictionary::String(strings(&["aaaa", "a", "aaaaaaaaaaaaaaaaa"]))
    );
    // Find a set of strings whose stream has an odd byte count.
    let mut found = false;
    for n in 1..40 {
        let items: Vec<String> = (0..n)
            .map(|i| format!("{}", (b'a' + (i % 5) as u8) as char))
            .collect();
        let (buffer, ..) = huffman_page(&items, Charset::Single(0));
        if buffer.len() % 2 == 1 {
            let bytes = string_dictionary(&[PageSpec::Huffman(items.clone(), Charset::Single(0))]);
            assert_eq!(
                parse_dictionary(&bytes, &DecodeLimits::default()).unwrap(),
                Dictionary::String(items)
            );
            found = true;
            break;
        }
    }
    assert!(found);
}

#[test]
fn huffman_table_rejects_bad_codes() {
    // Three 1-bit codes are over-subscribed.
    let mut encode = [0u8; 128];
    encode[0] = 0x11;
    encode[1] = 0x01;
    assert!(HuffmanTable::new(&encode).is_err());
    assert!(HuffmanTable::new(&encode[..100]).is_err());

    // An incomplete code fails when the stream hits an unused codeword.
    let mut encode = [0u8; 128];
    encode[0x30] = 0x02; // symbol 0x60 has a 2-bit code "00"
    let table = HuffmanTable::new(&encode).unwrap();
    let stream_bytes = [0x00, 0xC0]; // swapped: first byte read is 0xC0 = "11..."
    let stream = BitStream::new(&stream_bytes);
    let mut out = Vec::new();
    let err = table.decode(&stream, 0, 4, &mut out, 16).unwrap_err();
    assert!(corrupt_message(&err).contains("invalid codeword"), "{err}");
    // The symbol cap.
    let zeros = [0u8; 8];
    let stream = BitStream::new(&zeros);
    out.clear();
    let err = table.decode(&stream, 0, 64, &mut out, 10).unwrap_err();
    assert!(matches!(err, VertipaqError::LimitExceeded { .. }), "{err}");
    out.clear();
    table.decode(&stream, 0, 64, &mut out, 32).unwrap();
    assert_eq!(out, vec![0x60; 32]);

    // A page whose bit count exceeds its buffer.
    let mut bytes = string_dictionary(&[PageSpec::Huffman(strings(&["abc"]), Charset::Single(0))]);
    let pos = bytes
        .windows(4)
        .position(|w| w == [0xDD, 0xCC, 0xBB, 0xAA])
        .unwrap()
        + 4;
    bytes[pos..pos + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let err = parse_dictionary(&bytes, &DecodeLimits::default()).unwrap_err();
    assert!(corrupt_message(&err).contains("bits declared"), "{err}");
}

#[test]
fn dictionary_rejects_corrupt_input() {
    let limits = DecodeLimits::default();
    let mut invalid = long_dictionary(&[1], 8);
    invalid[..4].copy_from_slice(&(-1i32).to_le_bytes());
    assert!(matches!(
        parse_dictionary(&invalid, &limits),
        Err(VertipaqError::Corrupt { .. })
    ));
    invalid[..4].copy_from_slice(&7i32.to_le_bytes());
    assert!(matches!(
        parse_dictionary(&invalid, &limits),
        Err(VertipaqError::Unsupported { .. })
    ));
    assert!(parse_dictionary(&long_dictionary(&[1], 3), &limits).is_err());
    let mut real4 = real_dictionary(&[]);
    real4[36..40].copy_from_slice(&4u32.to_le_bytes());
    assert!(matches!(
        parse_dictionary(&real4, &limits),
        Err(VertipaqError::Unsupported { .. })
    ));

    let good = string_dictionary(&[
        PageSpec::Plain(strings(&["x", "yy"])),
        PageSpec::Huffman(strings(&["hello", "world"]), Charset::Single(0)),
    ]);
    assert!(parse_dictionary(&good, &limits).is_ok());
    for len in 0..good.len() {
        assert!(
            parse_dictionary(&good[..len], &limits).is_err(),
            "prefix {len}"
        );
    }
    // Every single-byte corruption fails or decodes without panicking.
    for i in 0..good.len() {
        let mut bytes = good.clone();
        bytes[i] ^= 0xA5;
        let _ = parse_dictionary(&bytes, &limits);
    }

    let handles = good.len() - 4 * 8 - 12;
    let mut bad_size = good.clone();
    bad_size[handles + 8..handles + 12].copy_from_slice(&4u32.to_le_bytes());
    assert!(
        corrupt_message(&parse_dictionary(&bad_size, &limits).unwrap_err()).contains("handle size")
    );
    let mut bad_page = good.clone();
    bad_page[handles + 12 + 4..handles + 12 + 8].copy_from_slice(&9u32.to_le_bytes());
    assert!(corrupt_message(&parse_dictionary(&bad_page, &limits).unwrap_err()).contains("page 9"));
    // More handles on a plain page than strings.
    let mut too_many = good.clone();
    too_many[handles + 12 + 3 * 8 + 4..handles + 12 + 3 * 8 + 8]
        .copy_from_slice(&0u32.to_le_bytes());
    assert!(parse_dictionary(&too_many, &limits).is_err());

    let mut mark = good.clone();
    let begin = mark
        .windows(4)
        .position(|w| w == [0xDD, 0xCC, 0xBB, 0xAA])
        .unwrap();
    mark[begin] = 0;
    assert!(parse_dictionary(&mark, &limits).is_err());
    let mut flag = good.clone();
    flag[begin - 1] = 2;
    assert!(
        corrupt_message(&parse_dictionary(&flag, &limits).unwrap_err())
            .contains("compression flag")
    );
    let mut used = good.clone();
    used[begin + 12..begin + 20].copy_from_slice(&1000u64.to_le_bytes());
    assert!(
        corrupt_message(&parse_dictionary(&used, &limits).unwrap_err()).contains("used characters")
    );
}

#[test]
fn dictionary_limits_are_enforced() {
    let limits = DecodeLimits {
        max_dictionary_entries: 2,
        ..DecodeLimits::default()
    };
    assert!(matches!(
        parse_dictionary(&long_dictionary(&[1, 2, 3], 8), &limits),
        Err(VertipaqError::LimitExceeded { .. })
    ));
    let limits = DecodeLimits {
        max_dictionary_bytes: 5,
        ..DecodeLimits::default()
    };
    let bytes = string_dictionary(&[PageSpec::Plain(strings(&["abc", "def"]))]);
    assert!(matches!(
        parse_dictionary(&bytes, &limits),
        Err(VertipaqError::LimitExceeded { .. })
    ));
    let bytes = string_dictionary(&[PageSpec::Huffman(strings(&["abc", "def"]), Charset::Multi)]);
    assert!(matches!(
        parse_dictionary(&bytes, &limits),
        Err(VertipaqError::LimitExceeded { .. })
    ));
}

// ---------------------------------------------------------------- convert

fn convert_ok(ids: &[u32], source: &Source<'_>, logical: &DataType) -> arrow::array::ArrayRef {
    let converted = convert::convert(ids, source, logical, u64::MAX).unwrap();
    assert_eq!(converted.invalid.count, 0, "{:?}", converted.invalid.first);
    converted.array
}

#[test]
fn value_encoding_uses_base_and_magnitude() {
    let ints = Source::Value {
        base_id: -2,
        magnitude: 1.0,
    };
    let array = convert_ok(&[3, 2, 5], &ints, &DataType::Int64);
    assert_eq!(
        array.as_primitive::<arrow::datatypes::Int64Type>(),
        &Int64Array::from(vec![Some(1), None, Some(3)])
    );
    let cents = Source::Value {
        base_id: 0,
        magnitude: 100.0,
    };
    let array = convert_ok(&[150, 3], &cents, &DataType::Double);
    assert_eq!(
        array.as_any().downcast_ref::<Float64Array>().unwrap(),
        &Float64Array::from(vec![1.5, 0.03])
    );
    let decimal = convert_ok(
        &[3, 2],
        &Source::Value {
            base_id: 12_342,
            magnitude: 1.0,
        },
        &DataType::Decimal,
    );
    assert_eq!(decimal.data_type(), &ArrowType::Decimal128(19, 4));
    let decimal = decimal.as_any().downcast_ref::<Decimal128Array>().unwrap();
    assert_eq!(decimal.value_as_string(0), "1.2345");
    assert!(decimal.is_null(1));
    let dates = convert_ok(
        &[3],
        &Source::Value {
            base_id: 42_002,
            magnitude: 1.0,
        },
        &DataType::DateTime,
    );
    assert_eq!(
        dates
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap()
            .value(0),
        1_420_070_400_000_000 // 2015-01-01
    );
    let flags = convert_ok(
        &[3, 4, 2],
        &Source::Value {
            base_id: -3,
            magnitude: 1.0,
        },
        &DataType::Boolean,
    );
    assert_eq!(
        flags.as_any().downcast_ref::<BooleanArray>().unwrap(),
        &BooleanArray::from(vec![Some(false), Some(true), None])
    );
    // Variant follows the storage.
    assert_eq!(
        convert_ok(&[3], &ints, &DataType::Variant).data_type(),
        &ArrowType::Int64
    );
    assert_eq!(
        convert_ok(&[3], &cents, &DataType::Variant).data_type(),
        &ArrowType::Float64
    );
    let text = convert_ok(&[3, 2], &ints, &DataType::String);
    assert_eq!(
        text.as_string::<i32>(),
        &StringArray::from(vec![Some("1"), None])
    );

    for magnitude in [0.0, f64::NAN, f64::INFINITY] {
        let bad = Source::Value {
            base_id: 0,
            magnitude,
        };
        assert!(convert::convert(&[3], &bad, &DataType::Int64, u64::MAX).is_err());
    }
    // Ids below the NULL id are invalid; an overflowing base too.
    let invalid = convert::convert(&[0, 1, 3], &ints, &DataType::Int64, u64::MAX).unwrap();
    assert_eq!(invalid.invalid.count, 2);
    let overflow = Source::Value {
        base_id: i64::MAX,
        magnitude: 1.0,
    };
    assert_eq!(
        convert::convert(&[3], &overflow, &DataType::Int64, u64::MAX)
            .unwrap()
            .invalid
            .count,
        1
    );
}

#[test]
fn hash_dictionaries_convert_to_logical_types() {
    let longs = Dictionary::Long(vec![12_345, -50_000, 0]);
    let source = Source::Dictionary(&longs);
    let ids = [3, 4, 2, 5];
    let decimal = convert_ok(&ids, &source, &DataType::Decimal);
    let decimal = decimal.as_any().downcast_ref::<Decimal128Array>().unwrap();
    assert_eq!(
        (0..4)
            .map(|i| decimal.is_valid(i).then(|| decimal.value_as_string(i)))
            .collect::<Vec<_>>(),
        vec![
            Some("1.2345".to_owned()),
            Some("-5.0000".to_owned()),
            None,
            Some("0.0000".to_owned())
        ]
    );
    let flags = convert_ok(&ids, &source, &DataType::Boolean);
    assert_eq!(
        flags.as_any().downcast_ref::<BooleanArray>().unwrap(),
        &BooleanArray::from(vec![Some(true), Some(true), None, Some(false)])
    );
    let doubles = convert_ok(&ids, &source, &DataType::Double);
    assert_eq!(
        doubles
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap()
            .value(1),
        -50_000.0
    );

    let reals = Dictionary::Real(vec![45_000.5, 25_569.0, 2.75]);
    let source = Source::Dictionary(&reals);
    let dates = convert_ok(&[3, 4, 2], &source, &DataType::DateTime);
    assert_eq!(
        dates.data_type(),
        &ArrowType::Timestamp(TimeUnit::Microsecond, None)
    );
    assert_eq!(
        dates
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap(),
        &TimestampMicrosecondArray::from(vec![Some(1_678_881_600_000_000), Some(0), None])
    );
    let ints = convert_ok(&[5], &source, &DataType::Int64);
    assert_eq!(
        ints.as_any().downcast_ref::<Int64Array>().unwrap().value(0),
        3
    );
    let nan = Dictionary::Real(vec![f64::NAN]);
    let bad =
        convert::convert(&[3], &Source::Dictionary(&nan), &DataType::Int64, u64::MAX).unwrap();
    assert_eq!(bad.invalid.count, 1);

    let texts = Dictionary::String(strings(&["", "x", "aGVsbG8=", "not base64!"]));
    let source = Source::Dictionary(&texts);
    let utf8 = convert_ok(&[4, 3, 2], &source, &DataType::String);
    assert_eq!(
        utf8.as_string::<i32>(),
        &StringArray::from(vec![Some("x"), Some(""), None])
    );
    // A string dictionary behind a numeric type stays text.
    assert_eq!(
        convert_ok(&[4], &source, &DataType::Int64).data_type(),
        &ArrowType::Utf8
    );
    let binary = convert::convert(&[5, 2, 6], &source, &DataType::Binary, u64::MAX).unwrap();
    assert_eq!(binary.invalid.count, 1);
    assert_eq!(
        binary.array.as_any().downcast_ref::<BinaryArray>().unwrap(),
        &BinaryArray::from(vec![Some(&b"hello"[..]), None, None])
    );

    // Ids outside the dictionary.
    let invalid = convert::convert(&[0, 1, 7, 3], &source, &DataType::String, u64::MAX).unwrap();
    assert_eq!(invalid.invalid.count, 3);
    assert_eq!(invalid.array.null_count(), 3);
    // Output size limit.
    assert!(matches!(
        convert::convert(&[4; 10], &source, &DataType::String, 5),
        Err(VertipaqError::LimitExceeded { .. })
    ));
}

#[test]
fn blank_columns_keep_their_type() {
    for (logical, arrow) in [
        (DataType::String, ArrowType::Utf8),
        (DataType::Int64, ArrowType::Int64),
        (DataType::Decimal, ArrowType::Decimal128(19, 4)),
        (
            DataType::DateTime,
            ArrowType::Timestamp(TimeUnit::Microsecond, None),
        ),
        (DataType::Variant, ArrowType::Utf8),
    ] {
        let converted = convert::convert(&[2, 2, 2], &Source::Blank, &logical, u64::MAX).unwrap();
        assert_eq!(converted.array.data_type(), &arrow);
        assert_eq!(converted.array.null_count(), 3);
        assert_eq!(converted.invalid.count, 0);
    }
    let odd = convert::convert(&[2, 3], &Source::Blank, &DataType::String, u64::MAX).unwrap();
    assert_eq!(odd.invalid.count, 1);
}

#[test]
fn ole_dates_and_base64() {
    assert_eq!(days_to_micros(25_569.0), Some(0));
    assert_eq!(days_to_micros(25_569.5), Some(43_200_000_000));
    assert_eq!(days_to_micros(0.0), Some(-2_209_161_600_000_000));
    assert_eq!(
        days_to_micros(-1.25),
        Some(-2_209_161_600_000_000 - 108_000_000_000)
    );
    assert_eq!(days_to_micros(f64::NAN), None);
    assert_eq!(days_to_micros(1e300), None);

    assert_eq!(base64_decode("").unwrap(), b"");
    assert_eq!(base64_decode("TWFu").unwrap(), b"Man");
    assert_eq!(base64_decode("TWE=").unwrap(), b"Ma");
    assert_eq!(base64_decode("TQ==").unwrap(), b"M");
    assert_eq!(base64_decode("T Q=\n=").unwrap(), b"M");
    assert!(base64_decode("TQ=a").is_none());
    assert!(base64_decode("T").is_none());
    assert!(base64_decode("@@@@").is_none());
}

// ---------------------------------------------------------------- end to end

/// A column of the synthetic model.
struct ColSpec {
    name: &'static str,
    /// TOM data type code.
    data_type: i64,
    /// `Column.Type` (1 data, 3 row number).
    kind: i64,
    /// `DictionaryStorage.Type` (1 hash, 2 value).
    encoding: i64,
    base_id: i64,
    magnitude: f64,
    dictionary: Option<Vec<u8>>,
    /// Data ids per partition.
    ids: Vec<Vec<u32>>,
    /// Replaces the `.idf` of the first partition.
    idf_override: Option<Vec<u8>>,
}

impl ColSpec {
    fn new(name: &'static str, data_type: i64, ids: Vec<Vec<u32>>) -> Self {
        Self {
            name,
            data_type,
            kind: 1,
            encoding: 1,
            base_id: 2,
            magnitude: 0.0,
            dictionary: None,
            ids,
            idf_override: None,
        }
    }

    fn value(mut self, base_id: i64, magnitude: f64) -> Self {
        self.encoding = 2;
        self.base_id = base_id;
        self.magnitude = magnitude;
        self
    }

    fn dictionary(mut self, bytes: Vec<u8>) -> Self {
        self.dictionary = Some(bytes);
        self
    }
}

/// `.idfmeta` + `.idf` for one partition of ids, encoded like the engine.
fn partition_files(ids: &[u32]) -> (Vec<u8>, Vec<u8>) {
    let has_nulls = ids.contains(&NULL_DATA_ID);
    let min = ids
        .iter()
        .copied()
        .filter(|&i| i != NULL_DATA_ID)
        .min()
        .unwrap_or(NULL_DATA_ID);
    let max = ids.iter().copied().max().unwrap_or(NULL_DATA_ID);
    let base = if has_nulls { NULL_DATA_ID } else { min };
    let width = (32 - (max - base).leading_zeros()).max(1) as u8;
    let mut spec = SegSpec::hybrid(ids.len() as u64, width, min, has_nulls);
    spec.max_data_id = max;
    (
        idfmeta_bytes(&[spec], false),
        encode_hybrid(ids, base, width, 64, 2),
    )
}

/// Builds an uncompressed ABF backup with `metadata.sqlitedb` and column files.
fn synthetic_backup(tables: &[(&str, Vec<u64>, Vec<ColSpec>)]) -> Vec<u8> {
    synthetic_builder(tables).build()
}

fn synthetic_builder(tables: &[(&str, Vec<u64>, Vec<ColSpec>)]) -> AbfBuilder {
    let mut sql = String::from(
        "CREATE TABLE Model (ID INTEGER, Name TEXT);
         CREATE TABLE [Table] (ID INTEGER, Name TEXT, SystemFlags INTEGER, TableStorageID INTEGER);
         CREATE TABLE [Column] (ID INTEGER, TableID INTEGER, ExplicitName TEXT, ExplicitDataType INTEGER,
             InferredDataType INTEGER, Type INTEGER, SourceColumn TEXT, ColumnStorageID INTEGER);
         CREATE TABLE [Partition] (ID INTEGER, TableID INTEGER, Name TEXT, Type INTEGER, Mode INTEGER,
             QueryDefinition TEXT, PartitionStorageID INTEGER);
         CREATE TABLE TableStorage (ID INTEGER, TableID INTEGER, Name TEXT, StorageFolderID INTEGER);
         CREATE TABLE PartitionStorage (ID INTEGER, PartitionID INTEGER, StoragePosition INTEGER,
             SegmentMapStorageID INTEGER, StorageFolderID INTEGER);
         CREATE TABLE SegmentMapStorage (ID INTEGER, PartitionStorageID INTEGER, Type INTEGER,
             RecordCount INTEGER, SegmentCount INTEGER, RecordsPerSegment INTEGER);
         CREATE TABLE ColumnStorage (ID INTEGER, ColumnID INTEGER, Name TEXT, StoragePosition INTEGER,
             DictionaryStorageID INTEGER);
         CREATE TABLE ColumnPartitionStorage (ID INTEGER, ColumnStorageID INTEGER,
             PartitionStorageID INTEGER, SegmentStorageID INTEGER, StorageFileID INTEGER);
         CREATE TABLE SegmentStorage (ID INTEGER, ColumnPartitionStorageID INTEGER,
             SegmentCount INTEGER, StorageFileID INTEGER);
         CREATE TABLE DictionaryStorage (ID INTEGER, ColumnStorageID INTEGER, Type INTEGER,
             DataType INTEGER, BaseId INTEGER, Magnitude REAL, IsOperatingOn32 INTEGER,
             StorageFileID INTEGER);
         CREATE TABLE StorageFile (ID INTEGER, StorageFolderID INTEGER, FileName TEXT);
         CREATE TABLE StorageFolder (ID INTEGER, Path TEXT);
         INSERT INTO Model VALUES (1, 'Model');\n",
    );
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut next_id = 100i64;
    let mut id = || {
        next_id += 1;
        next_id
    };
    for (table_name, partition_rows, columns) in tables {
        let (table_id, storage_id, folder_id) = (id(), id(), id());
        let folder = format!("T{table_id}.tbl");
        sql += &format!(
            "INSERT INTO [Table] VALUES ({table_id}, '{table_name}', 0, {storage_id});
             INSERT INTO TableStorage VALUES ({storage_id}, {table_id}, 'T ({table_id})', {folder_id});
             INSERT INTO StorageFolder VALUES ({folder_id}, '{folder}');\n"
        );
        let mut partition_storage = Vec::new();
        for (position, rows) in partition_rows.iter().enumerate() {
            let (pid, psid, smid, pfolder) = (id(), id(), id(), id());
            sql += &format!(
                "INSERT INTO [Partition] VALUES ({pid}, {table_id}, 'p{position}', 4, 0, 'let x = 1 in x', {psid});
                 INSERT INTO PartitionStorage VALUES ({psid}, {pid}, {position}, {smid}, {pfolder});
                 INSERT INTO SegmentMapStorage VALUES ({smid}, {psid}, 3, {rows}, 1, {rows});
                 INSERT INTO StorageFolder VALUES ({pfolder}, '{folder}\\{pid}.prt');\n"
            );
            partition_storage.push((psid, pfolder, format!("{folder}\\{pid}.prt")));
        }
        for (position, column) in columns.iter().enumerate() {
            let (cid, csid, dsid) = (id(), id(), id());
            sql += &format!(
                "INSERT INTO [Column] VALUES ({cid}, {table_id}, '{}', {}, {}, {}, '{}', {csid});
                 INSERT INTO ColumnStorage VALUES ({csid}, {cid}, '{} ({cid})', {position}, {dsid});\n",
                column.name, column.data_type, column.data_type, column.kind, column.name, column.name
            );
            let dictionary_file = match &column.dictionary {
                Some(bytes) => {
                    let fid = id();
                    let name = format!("{}.dictionary", column.name);
                    sql += &format!(
                        "INSERT INTO StorageFile VALUES ({fid}, {folder_id}, '{name}');\n"
                    );
                    files.push((format!("{folder}\\{name}"), bytes.clone()));
                    fid
                }
                None => 0,
            };
            let physical = match column.data_type {
                2 => 2,
                9 | 8 => 8,
                _ => 6,
            };
            sql += &format!(
                "INSERT INTO DictionaryStorage VALUES ({dsid}, {csid}, {}, {physical}, {}, {}, 0, {dictionary_file});\n",
                column.encoding, column.base_id, column.magnitude
            );
            for (p, ids) in column.ids.iter().enumerate() {
                let (psid, pfolder, pfolder_path) = &partition_storage[p];
                let (cpsid, ssid, idf_id, meta_id) = (id(), id(), id(), id());
                let (meta, mut idf) = partition_files(ids);
                if p == 0
                    && let Some(bytes) = &column.idf_override
                {
                    idf = bytes.clone();
                }
                let idf_name = format!("{}.{p}.idf", column.name);
                sql += &format!(
                    "INSERT INTO ColumnPartitionStorage VALUES ({cpsid}, {csid}, {psid}, {ssid}, {idf_id});
                     INSERT INTO SegmentStorage VALUES ({ssid}, {cpsid}, 1, {meta_id});
                     INSERT INTO StorageFile VALUES ({idf_id}, {pfolder}, '{idf_name}'),
                         ({meta_id}, {pfolder}, '{idf_name}meta');\n"
                );
                files.push((format!("{pfolder_path}\\{idf_name}"), idf));
                files.push((format!("{pfolder_path}\\{idf_name}meta"), meta));
            }
        }
    }
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&sql).unwrap();
    let db = conn.serialize("main").unwrap().to_vec();
    let mut builder = AbfBuilder::new().file("metadata.sqlitedb", db);
    for (path, bytes) in files {
        builder = builder.file(&path, bytes);
    }
    builder
}

fn sample_tables() -> Vec<(&'static str, Vec<u64>, Vec<ColSpec>)> {
    let names = string_dictionary(&[
        PageSpec::Plain(strings(&["", "Alpha"])),
        PageSpec::Huffman(strings(&["café", "Zürich"]), Charset::Single(0)),
        PageSpec::Huffman(strings(&["Привет"]), Charset::Single(0x04)),
        PageSpec::Huffman(strings(&["日本語😀"]), Charset::Multi),
    ]);
    let mut row_number = ColSpec::new("RowNumber-2662979B", 6, vec![]);
    row_number.kind = 3;
    let mut broken = ColSpec::new("Bad", 6, vec![vec![3, 3]]).dictionary(long_dictionary(&[1], 8));
    broken.idf_override = Some(vec![1, 2, 3]);
    vec![
        (
            "Sales",
            vec![3, 2],
            vec![
                row_number,
                ColSpec::new("Id", 6, vec![vec![3, 4, 5], vec![2, 6]]).value(-2, 1.0),
                ColSpec::new("Name", 2, vec![vec![8, 3, 4], vec![2, 7]]).dictionary(names),
                ColSpec::new("Price", 10, vec![vec![3, 4, 3], vec![4, 3]])
                    .dictionary(long_dictionary(&[12_345, -50_000], 8)),
                ColSpec::new("When", 9, vec![vec![3, 4, 3], vec![3, 3]])
                    .dictionary(real_dictionary(&[45_000.5, 25_569.0])),
                ColSpec::new("Flag", 11, vec![vec![4, 3, 4], vec![3, 2]]).value(-3, 1.0),
                ColSpec::new("Ratio", 8, vec![vec![150, 3, 1000], vec![2, 250]]).value(0, 100.0),
                ColSpec::new("Empty", 2, vec![vec![2, 2, 2], vec![2, 2]]),
            ],
        ),
        (
            "Broken",
            vec![2],
            vec![
                broken,
                ColSpec::new("Good", 6, vec![vec![3, 4]]).value(0, 1.0),
            ],
        ),
    ]
}

#[test]
fn compressed_backup_decodes_like_an_uncompressed_one() {
    // Power Pivot style: every model file framed as Xpress chunks, CRC end markers.
    let tables = sample_tables();
    let plain = AbfArchive::parse(synthetic_backup(&tables)).unwrap();
    let packed = AbfArchive::parse(
        synthetic_builder(&tables)
            .apply_compression()
            .crc_markers()
            .build(),
    )
    .unwrap();
    assert!(packed.is_compressed());
    let limits = DecodeLimits::default();
    let a = load_model(&plain, &ModelOptions::default()).unwrap();
    let b = load_model(&packed, &ModelOptions::default()).unwrap();
    assert_eq!(a.model, b.model);
    let da = decode_all_with_diagnostics(&plain, &a.storage, &a.model, &limits);
    let db = decode_all_with_diagnostics(&packed, &b.storage, &b.model, &limits);
    assert!(!da.tables.is_empty());
    assert_eq!(da.tables, db.tables);
    assert_eq!(da.diagnostics, db.diagnostics);
}

#[test]
fn synthetic_model_decodes_end_to_end() {
    let backup = synthetic_backup(&sample_tables());
    let archive = AbfArchive::parse(backup.clone()).unwrap();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    let limits = DecodeLimits::default();

    let sales = decode_table(&archive, &loaded.storage, &loaded.model, "sales", &limits).unwrap();
    assert_eq!(sales.num_rows(), 5);
    let names: Vec<&str> = sales
        .schema_ref()
        .fields()
        .iter()
        .map(|f| f.name().as_str())
        .collect();
    assert_eq!(
        names,
        ["Id", "Name", "Price", "When", "Flag", "Ratio", "Empty"]
    );

    let id = sales
        .column(0)
        .as_primitive::<arrow::datatypes::Int64Type>();
    assert_eq!(
        id,
        &Int64Array::from(vec![Some(1), Some(2), Some(3), None, Some(4)])
    );
    let name = sales.column(1).as_string::<i32>();
    assert_eq!(
        name,
        &StringArray::from(vec![
            Some("日本語😀"),
            Some(""),
            Some("Alpha"),
            None,
            Some("Привет")
        ])
    );
    let price = sales
        .column(2)
        .as_any()
        .downcast_ref::<Decimal128Array>()
        .unwrap();
    assert_eq!(price.data_type(), &ArrowType::Decimal128(19, 4));
    assert_eq!(
        price.values().to_vec(),
        vec![12_345, -50_000, 12_345, -50_000, 12_345]
    );
    let when = sales
        .column(3)
        .as_any()
        .downcast_ref::<TimestampMicrosecondArray>()
        .unwrap();
    assert_eq!(when.value(0), 1_678_881_600_000_000);
    assert_eq!(when.value(1), 0);
    let flag = sales
        .column(4)
        .as_any()
        .downcast_ref::<BooleanArray>()
        .unwrap();
    assert_eq!(
        flag,
        &BooleanArray::from(vec![Some(true), Some(false), Some(true), Some(false), None])
    );
    let ratio = sales
        .column(5)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    assert_eq!(
        ratio,
        &Float64Array::from(vec![Some(1.5), Some(0.03), Some(10.0), None, Some(2.5)])
    );
    assert_eq!(sales.column(6).data_type(), &ArrowType::Utf8);
    assert_eq!(sales.column(6).null_count(), 5);

    // Single columns, by storage name lookup (case-insensitive).
    let column = decode_column(&archive, &loaded.storage, "SALES", "name", &limits).unwrap();
    assert_eq!(column.len(), 5);
    let err = decode_column(
        &archive,
        &loaded.storage,
        "Sales",
        "RowNumber-2662979B",
        &limits,
    )
    .unwrap_err();
    assert!(
        matches!(err.innermost(), VertipaqError::Unsupported { .. }),
        "{err}"
    );

    // Strict decoding fails on the broken table, lenient decoding reports it.
    let err =
        decode_table(&archive, &loaded.storage, &loaded.model, "Broken", &limits).unwrap_err();
    assert!(matches!(err, VertipaqError::InColumn { .. }), "{err}");
    assert!(decode_all(&archive, &loaded.storage, &loaded.model, &limits).is_err());
    let decoded = decode_all_with_diagnostics(&archive, &loaded.storage, &loaded.model, &limits);
    assert_eq!(decoded.tables.len(), 2);
    assert_eq!(decoded.diagnostics.len(), 1, "{:#?}", decoded.diagnostics);
    assert_eq!(decoded.diagnostics[0].code, codes::COLUMN_FAILED);
    assert!(decoded.diagnostics[0].object.is_some());
    let broken = &decoded.tables[1].1;
    assert_eq!(broken.column(0).null_count(), 2);
    assert_eq!(broken.column(0).data_type(), &ArrowType::Int64);
    assert_eq!(broken.column(1).null_count(), 0);

    // The whole pipeline from (uncompressed) DataModel bytes.
    let (model, tables, diagnostics) = open_pbix_data(&backup, &limits).unwrap();
    assert_eq!(model.tables.len(), 2);
    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].1, sales);
    assert_eq!(diagnostics.len(), 1);

    // A total budget smaller than the first table stops decoding.
    let tight = DecodeLimits {
        max_total_bytes: 1,
        ..limits
    };
    let decoded = decode_all_with_diagnostics(&archive, &loaded.storage, &loaded.model, &tight);
    assert_eq!(decoded.tables.len(), 1);
    assert!(
        decoded
            .diagnostics
            .iter()
            .any(|d| d.code == codes::BUDGET_EXHAUSTED)
    );
    assert!(matches!(
        decode_all(&archive, &loaded.storage, &loaded.model, &tight),
        Err(VertipaqError::LimitExceeded { .. })
    ));
}

#[test]
fn invalid_ids_follow_the_policy() {
    let mut tables = sample_tables();
    tables.truncate(1);
    // Name gets an id past its 6 dictionary entries.
    tables[0].2[2].ids[1] = vec![2, 99];
    let archive = AbfArchive::parse(synthetic_backup(&tables)).unwrap();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();

    let strict = DecodeLimits::default();
    let err = decode_column(&archive, &loaded.storage, "Sales", "Name", &strict).unwrap_err();
    assert!(
        matches!(
            err.innermost(),
            VertipaqError::InvalidDataIds { count: 1, .. }
        ),
        "{err}"
    );
    let blank = strict.with_invalid_ids(InvalidIdPolicy::Blank);
    let decoded =
        decode_column_with_diagnostics(&archive, &loaded.storage, "Sales", "Name", &blank).unwrap();
    assert_eq!(decoded.invalid_rows, 1);
    assert_eq!(decoded.array.null_count(), 2);
    assert_eq!(decoded.diagnostics[0].code, codes::INVALID_IDS);
    let all = decode_all_with_diagnostics(&archive, &loaded.storage, &loaded.model, &blank);
    assert_eq!(all.diagnostics.len(), 1);
    assert_eq!(all.diagnostics[0].code, codes::INVALID_IDS);
}

#[test]
fn tables_without_any_decodable_column_are_skipped() {
    let mut tables = sample_tables();
    tables.remove(0);
    // Both columns of "Broken" fail now.
    tables[0].2[1].idf_override = Some(vec![9]);
    let archive = AbfArchive::parse(synthetic_backup(&tables)).unwrap();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    let decoded = decode_all_with_diagnostics(
        &archive,
        &loaded.storage,
        &loaded.model,
        &DecodeLimits::default(),
    );
    assert!(decoded.tables.is_empty());
    assert_eq!(decoded.diagnostics.len(), 1, "{:#?}", decoded.diagnostics);
    assert_eq!(decoded.diagnostics[0].code, codes::TABLE_FAILED);
}

#[test]
fn partition_row_counts_must_match() {
    let mut tables = sample_tables();
    tables.truncate(1);
    // Id's first partition declares 3 rows in the metadata but 2 in its .idfmeta.
    tables[0].2[1].ids[0] = vec![3, 4];
    let archive = AbfArchive::parse(synthetic_backup(&tables)).unwrap();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    let err = decode_column(
        &archive,
        &loaded.storage,
        "Sales",
        "Id",
        &DecodeLimits::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("segments hold 2 rows"), "{err}");
}

#[test]
fn tables_without_rows_decode_to_empty_batches() {
    // A calculated table saved before it was ever computed: one partition of
    // zero rows, a hash column without a dictionary file.
    let tables = vec![(
        "Pending",
        vec![0],
        vec![
            ColSpec::new("Name", 2, vec![vec![]]),
            ColSpec::new("Count", 6, vec![vec![]]).value(0, 1.0),
        ],
    )];
    let archive = AbfArchive::parse(synthetic_backup(&tables)).unwrap();
    let loaded = load_model(&archive, &ModelOptions::default()).unwrap();
    let limits = DecodeLimits::default();
    let batch = decode_table(&archive, &loaded.storage, &loaded.model, "Pending", &limits).unwrap();
    assert_eq!((batch.num_rows(), batch.num_columns()), (0, 2));
    assert_eq!(batch.column(0).data_type(), &ArrowType::Utf8);
    assert_eq!(batch.column(1).data_type(), &ArrowType::Int64);
    let decoded = decode_all_with_diagnostics(&archive, &loaded.storage, &loaded.model, &limits);
    assert!(decoded.diagnostics.is_empty(), "{:#?}", decoded.diagnostics);
    assert_eq!(decoded.tables.len(), 1);
    assert_eq!(decoded.tables[0].1.num_rows(), 0);
}

// ---------------------------------------------------------------- fuzz

#[test]
fn fuzz_entry_point_never_panics() {
    let mut rng = Rng(0xF00D);
    let mut seeds: Vec<Vec<u8>> = vec![
        Vec::new(),
        vec![0xFF; 64],
        idfmeta_bytes(
            &[SegSpec::hybrid(70, 3, 3, true), SegSpec::packed(9, 5, 4)],
            true,
        ),
        encode_hybrid(&[3, 4, 5, 3, 3, 9], 3, 3, 64, 2),
        long_dictionary(&[1, 2, 3], 4),
        real_dictionary(&[1.5, f64::NAN]),
        string_dictionary(&[
            PageSpec::Plain(strings(&["a", "bc"])),
            PageSpec::Huffman(strings(&["hello", "wörld"]), Charset::Single(0)),
            PageSpec::Huffman(strings(&["日本"]), Charset::Multi),
        ]),
        synthetic_backup(&sample_tables()),
    ];
    let mut huffman = vec![0u8];
    huffman.extend_from_slice(&[0x44; 128]);
    huffman.extend((0..200).map(|i| i as u8));
    seeds.push(huffman);
    for seed in &seeds {
        fuzz_vertipaq(seed);
        let rounds = if seed.len() > 10_000 { 40 } else { 300 };
        for _ in 0..rounds {
            let mut data = seed.clone();
            if !data.is_empty() {
                for _ in 0..=rng.below(4) {
                    let i = rng.below(data.len() as u64) as usize;
                    data[i] = rng.next() as u8;
                }
                if rng.below(4) == 0 {
                    let cut = rng.below(data.len() as u64) as usize;
                    data.truncate(cut);
                }
            }
            fuzz_vertipaq(&data);
        }
    }
    for _ in 0..500 {
        let len = rng.below(400) as usize;
        let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        fuzz_vertipaq(&data);
    }
}
