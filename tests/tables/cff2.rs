use std::fmt::Write;

use ttf_parser::{cff2, CFFError, GlyphId, Rect};

struct Builder(String);

impl ttf_parser::OutlineBuilder for Builder {
    fn move_to(&mut self, x: f32, y: f32) {
        write!(&mut self.0, "M {} {} ", x, y).unwrap();
    }

    fn line_to(&mut self, x: f32, y: f32) {
        write!(&mut self.0, "L {} {} ", x, y).unwrap();
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        write!(&mut self.0, "Q {} {} {} {} ", x1, y1, x, y).unwrap();
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        write!(&mut self.0, "C {} {} {} {} {} {} ", x1, y1, x2, y2, x, y).unwrap();
    }

    fn close(&mut self) {
        write!(&mut self.0, "Z ").unwrap();
    }
}

// https://docs.microsoft.com/en-us/typography/opentype/spec/cff2charstr#4-charstring-operators
#[allow(dead_code)]
mod operator {
    pub const VERTICAL_MOVE_TO: u8       = 4;
    pub const LINE_TO: u8                = 5;
    pub const CALL_LOCAL_SUBROUTINE: u8  = 10;
    pub const ENDCHAR: u8                = 14; // reserved in CFF2
    pub const VS_INDEX: u8               = 15;
    pub const BLEND: u8                  = 16;
    pub const HORIZONTAL_MOVE_TO: u8     = 22;
    pub const CALL_GLOBAL_SUBROUTINE: u8 = 29;
}

// https://docs.microsoft.com/en-us/typography/opentype/spec/cff2#table-9-top-dict-operator-entries
mod top_dict_operator {
    pub const CHAR_STRINGS_OFFSET: u8 = 17;
    pub const VARIATION_STORE_OFFSET: u8 = 24;
    // Two-byte operator: 12 36.
    pub const FONT_DICT_INDEX_OFFSET: [u8; 2] = [12, 36];
}

// https://docs.microsoft.com/en-us/typography/opentype/spec/cff2#table-10-font-dict-operator-entries
mod font_dict_operator {
    pub const PRIVATE_DICT_SIZE_AND_OFFSET: u8 = 18;
}

// https://docs.microsoft.com/en-us/typography/opentype/spec/cff2#table-16-private-dict-operators
mod private_dict_operator {
    pub const LOCAL_SUBROUTINES_OFFSET: u8 = 19;
}

// Mirrors `cff2::MAX_SUBROUTINE_CALLS`, which is private.
const MAX_SUBROUTINE_CALLS: u32 = 4_096;

// A charstring integer operand. Adobe Technical Note #5177, Table 3.
fn cs_int(value: i32) -> Vec<u8> {
    match value {
        -107..=107 => vec![(value + 139) as u8],
        108..=1131 => {
            let n = value - 108;
            vec![((n >> 8) + 247) as u8, (n & 0xFF) as u8]
        }
        -1131..=-108 => {
            let n = -value - 108;
            vec![((n >> 8) + 251) as u8, (n & 0xFF) as u8]
        }
        _ => {
            let mut v = vec![28];
            v.extend_from_slice(&(value as i16).to_be_bytes());
            v
        }
    }
}

// A DICT integer operand. Always uses the 5-byte form so that every DICT this
// module builds has a size that is known before the offsets it contains are. ~keep
fn dict_int(value: i32) -> Vec<u8> {
    let mut v = vec![29];
    v.extend_from_slice(&value.to_be_bytes());
    v
}

// A CFF2 INDEX. Identical to a CFF1 INDEX except that `count` is a u32.
fn index(objects: &[Vec<u8>]) -> Vec<u8> {
    if objects.is_empty() {
        return vec![0, 0, 0, 0];
    }

    let mut offsets = Vec::with_capacity(objects.len() + 1);
    let mut current = 1usize;
    offsets.push(current);
    for object in objects {
        current += object.len();
        offsets.push(current);
    }

    let last = *offsets.last().unwrap();
    let offset_size = if last <= 0xFF {
        1
    } else if last <= 0xFFFF {
        2
    } else if last <= 0xFF_FFFF {
        3
    } else {
        4
    };

    let mut out = Vec::new();
    out.extend_from_slice(&(objects.len() as u32).to_be_bytes());
    out.push(offset_size as u8);
    for offset in offsets {
        for shift in (0..offset_size).rev() {
            out.push(((offset >> (shift * 8)) & 0xFF) as u8);
        }
    }
    for object in objects {
        out.extend_from_slice(object);
    }
    out
}

// An ItemVariationStore with `regions` regions and a single ItemVariationData
// that references all of them, preceded by the u16 length field CFF2 requires.
fn variation_store(regions: u16) -> Vec<u8> {
    const HEADER_LEN: usize = 12; // format + regionListOffset + count + one offset
    let region_list_len = 4 + 6 * usize::from(regions);
    let region_list_offset = HEADER_LEN;
    let variation_data_offset = HEADER_LEN + region_list_len;

    let mut store = Vec::new();
    store.extend_from_slice(&1u16.to_be_bytes()); // format
    store.extend_from_slice(&(region_list_offset as u32).to_be_bytes());
    store.extend_from_slice(&1u16.to_be_bytes()); // itemVariationDataCount
    store.extend_from_slice(&(variation_data_offset as u32).to_be_bytes());

    // VariationRegionList
    store.extend_from_slice(&1u16.to_be_bytes()); // axisCount
    store.extend_from_slice(&regions.to_be_bytes()); // regionCount
    for _ in 0..regions {
        store.extend_from_slice(&0i16.to_be_bytes()); // startCoord
        store.extend_from_slice(&0i16.to_be_bytes()); // peakCoord
        store.extend_from_slice(&0i16.to_be_bytes()); // endCoord
    }

    // ItemVariationData
    store.extend_from_slice(&0u16.to_be_bytes()); // itemCount
    store.extend_from_slice(&0u16.to_be_bytes()); // wordDeltaCount
    store.extend_from_slice(&regions.to_be_bytes()); // regionIndexCount
    for i in 0..regions {
        store.extend_from_slice(&i.to_be_bytes());
    }

    let mut out = Vec::new();
    out.extend_from_slice(&(store.len() as u16).to_be_bytes());
    out.extend_from_slice(&store);
    out
}

/// Builds a complete CFF2 table.
///
/// Layout: header, Top DICT, Global Subr INDEX, VariationStore, FDArray,
/// Private DICT, Local Subr INDEX, CharStrings INDEX.
struct Cff2 {
    /// `None` omits the Top DICT `vstore` entry entirely, which the CFF2 spec allows:
    /// a static CFF2 font has no variation data.
    regions: Option<u16>,
    global_subrs: Vec<Vec<u8>>,
    local_subrs: Vec<Vec<u8>>,
    char_strings: Vec<Vec<u8>>,
}

impl Cff2 {
    fn new(char_strings: Vec<Vec<u8>>) -> Self {
        Cff2 {
            regions: Some(1),
            global_subrs: Vec::new(),
            local_subrs: Vec::new(),
            char_strings,
        }
    }

    fn build(&self) -> Vec<u8> {
        const HEADER_LEN: usize = 5;
        // dict_int is 5 bytes, so: charstrings 6, vstore 6, fdarray 7.
        const VSTORE_ENTRY_LEN: usize = 6;
        let top_dict_len = if self.regions.is_some() { 19 } else { 19 - VSTORE_ENTRY_LEN };
        // `size offset 18`
        const FONT_DICT_LEN: usize = 11;
        // count(4) + offSize(1) + 2 one-byte offsets + payload
        const FD_ARRAY_LEN: usize = 4 + 1 + 2 + FONT_DICT_LEN;
        // `offset 19`
        const PRIVATE_DICT_LEN: usize = 6;

        let global_subrs = index(&self.global_subrs);
        let local_subrs = index(&self.local_subrs);
        let variation_store = self.regions.map(variation_store).unwrap_or_default();
        let char_strings = index(&self.char_strings);

        let global_subrs_offset = HEADER_LEN + top_dict_len;
        let variation_store_offset = global_subrs_offset + global_subrs.len();
        let font_dict_index_offset = variation_store_offset + variation_store.len();
        let private_dict_offset = font_dict_index_offset + FD_ARRAY_LEN;
        let local_subrs_offset = private_dict_offset + PRIVATE_DICT_LEN;
        let char_strings_offset = local_subrs_offset + local_subrs.len();

        let mut top_dict = Vec::new();
        top_dict.extend_from_slice(&dict_int(char_strings_offset as i32));
        top_dict.push(top_dict_operator::CHAR_STRINGS_OFFSET);
        if self.regions.is_some() {
            top_dict.extend_from_slice(&dict_int(variation_store_offset as i32));
            top_dict.push(top_dict_operator::VARIATION_STORE_OFFSET);
        }
        top_dict.extend_from_slice(&dict_int(font_dict_index_offset as i32));
        top_dict.extend_from_slice(&top_dict_operator::FONT_DICT_INDEX_OFFSET);
        assert_eq!(top_dict.len(), top_dict_len);

        let mut font_dict = Vec::new();
        font_dict.extend_from_slice(&dict_int(PRIVATE_DICT_LEN as i32));
        font_dict.extend_from_slice(&dict_int(private_dict_offset as i32));
        font_dict.push(font_dict_operator::PRIVATE_DICT_SIZE_AND_OFFSET);
        assert_eq!(font_dict.len(), FONT_DICT_LEN);
        let font_dict_index = index(&[font_dict]);
        assert_eq!(font_dict_index.len(), FD_ARRAY_LEN);

        // 'The local subroutines offset is relative to the beginning of the Private DICT data.'
        let mut private_dict = Vec::new();
        private_dict.extend_from_slice(&dict_int(PRIVATE_DICT_LEN as i32));
        private_dict.push(private_dict_operator::LOCAL_SUBROUTINES_OFFSET);
        assert_eq!(private_dict.len(), PRIVATE_DICT_LEN);

        let mut data = Vec::new();
        data.push(2); // majorVersion
        data.push(0); // minorVersion
        data.push(HEADER_LEN as u8); // headerSize
        data.extend_from_slice(&(top_dict_len as u16).to_be_bytes()); // topDictLength
        data.extend_from_slice(&top_dict);
        assert_eq!(data.len(), global_subrs_offset);
        data.extend_from_slice(&global_subrs);
        assert_eq!(data.len(), variation_store_offset);
        data.extend_from_slice(&variation_store);
        assert_eq!(data.len(), font_dict_index_offset);
        data.extend_from_slice(&font_dict_index);
        assert_eq!(data.len(), private_dict_offset);
        data.extend_from_slice(&private_dict);
        assert_eq!(data.len(), local_subrs_offset);
        data.extend_from_slice(&local_subrs);
        assert_eq!(data.len(), char_strings_offset);
        data.extend_from_slice(&char_strings);
        data
    }
}

fn outline(data: &[u8]) -> (Result<Rect, CFFError>, String) {
    let table = cff2::Table::parse(data).unwrap();
    let mut builder = Builder(String::new());
    let result = table.outline(&[], GlyphId(0), &mut builder);
    (result, builder.0)
}

fn rect(x_min: i16, y_min: i16, x_max: i16, y_max: i16) -> Rect {
    Rect { x_min, y_min, x_max, y_max }
}

fn call_subr(index: usize, subrs_len: usize, operator: u8) -> Vec<u8> {
    let bias = if subrs_len < 1240 {
        107
    } else if subrs_len < 33900 {
        1131
    } else {
        32768
    };
    let mut out = cs_int(index as i32 - bias);
    out.push(operator);
    out
}

/// One glyph and a chain of `depth` subroutines where subr[i] calls subr[i+1]
/// `fanout` times. The deepest subroutine draws a line, so a font that does not
/// amplify produces a real outline. CFF2 subroutines have no `return` operator.
fn fanout_font(fanout: usize, depth: usize, local: bool) -> Vec<u8> {
    let operator = if local {
        operator::CALL_LOCAL_SUBROUTINE
    } else {
        operator::CALL_GLOBAL_SUBROUTINE
    };

    let mut subrs = Vec::new();
    for level in 0..depth {
        let mut body = Vec::new();
        if level + 1 < depth {
            for _ in 0..fanout {
                body.extend_from_slice(&call_subr(level + 1, depth, operator));
            }
        } else {
            body.extend_from_slice(&cs_int(50));
            body.extend_from_slice(&cs_int(50));
            body.push(operator::LINE_TO);
        }
        subrs.push(body);
    }

    let mut root = Vec::new();
    root.extend_from_slice(&cs_int(100));
    root.push(operator::HORIZONTAL_MOVE_TO);
    for _ in 0..fanout {
        root.extend_from_slice(&call_subr(0, depth, operator));
    }

    let mut font = Cff2::new(vec![root]);
    if local {
        font.local_subrs = subrs;
    } else {
        font.global_subrs = subrs;
    }
    font.build()
}

#[test]
fn minimal_glyph_outlines() {
    let mut char_string = Vec::new();
    char_string.extend_from_slice(&cs_int(100));
    char_string.push(operator::HORIZONTAL_MOVE_TO);
    char_string.extend_from_slice(&cs_int(50));
    char_string.extend_from_slice(&cs_int(50));
    char_string.push(operator::LINE_TO);

    let (result, path) = outline(&Cff2::new(vec![char_string]).build());

    // CFF2 has no `endchar`; the last contour is closed when the charstring ends.
    assert_eq!(path, "M 100 0 L 150 50 Z ");
    assert_eq!(result.unwrap(), rect(100, 0, 150, 50));
}

// Every contour must be closed, the last one included: CFF1 closes it on `endchar`, which CFF2
// lacks. Rasterizers that don't close paths themselves fill an open last contour solid.
#[test]
fn every_contour_is_closed() {
    let mut char_string = Vec::new();
    char_string.extend_from_slice(&cs_int(100));
    char_string.push(operator::HORIZONTAL_MOVE_TO);
    char_string.extend_from_slice(&cs_int(50));
    char_string.extend_from_slice(&cs_int(50));
    char_string.push(operator::LINE_TO);
    char_string.extend_from_slice(&cs_int(100));
    char_string.push(operator::VERTICAL_MOVE_TO);
    char_string.extend_from_slice(&cs_int(-50));
    char_string.extend_from_slice(&cs_int(50));
    char_string.push(operator::LINE_TO);

    let (result, path) = outline(&Cff2::new(vec![char_string]).build());

    assert_eq!(path, "M 100 0 L 150 50 Z M 150 150 L 100 200 Z ");
    assert_eq!(result.unwrap(), rect(100, 0, 150, 200));
}

// Regression test for https://github.com/harfbuzz/ttf-parser/issues/239. The `vstore` Top DICT
// entry is optional, but `parse_char_string` used to resolve variation index 0 before running
// a single operator, which fails on an absent store — so a static CFF2 font could not outline
// any glyph at all. The output must match `minimal_glyph_outlines` exactly.
#[test]
fn glyph_without_a_variation_store_outlines() {
    let mut char_string = Vec::new();
    char_string.extend_from_slice(&cs_int(100));
    char_string.push(operator::HORIZONTAL_MOVE_TO);
    char_string.extend_from_slice(&cs_int(50));
    char_string.extend_from_slice(&cs_int(50));
    char_string.push(operator::LINE_TO);

    let mut font = Cff2::new(vec![char_string]);
    font.regions = None;
    let (result, path) = outline(&font.build());

    assert_eq!(path, "M 100 0 L 150 50 Z ");
    assert_eq!(result.unwrap(), rect(100, 0, 150, 50));
}

// `blend` genuinely needs the store, so it must still be rejected when there is none.
#[test]
fn blend_without_a_variation_store_is_rejected() {
    let mut char_string = Vec::new();
    char_string.extend_from_slice(&cs_int(100));
    char_string.extend_from_slice(&cs_int(10));
    char_string.extend_from_slice(&cs_int(1));
    char_string.push(operator::BLEND);
    char_string.push(operator::HORIZONTAL_MOVE_TO);

    let mut font = Cff2::new(vec![char_string]);
    font.regions = None;
    let (result, _) = outline(&font.build());

    assert_eq!(result.unwrap_err(), CFFError::InvalidItemVariationDataIndex);
}

#[test]
fn blend_applies_region_deltas_to_its_operands() {
    // `100 10 1 blend` with one region whose scalar evaluates to 1.0 leaves 110
    // on the stack, which `hmoveto` then consumes.
    let mut char_string = Vec::new();
    char_string.extend_from_slice(&cs_int(100));
    char_string.extend_from_slice(&cs_int(10));
    char_string.extend_from_slice(&cs_int(1));
    char_string.push(operator::BLEND);
    char_string.push(operator::HORIZONTAL_MOVE_TO);
    char_string.extend_from_slice(&cs_int(50));
    char_string.extend_from_slice(&cs_int(50));
    char_string.push(operator::LINE_TO);

    let (result, path) = outline(&Cff2::new(vec![char_string]).build());

    assert_eq!(path, "M 110 0 L 160 50 Z ");
    assert_eq!(result.unwrap(), rect(110, 0, 160, 50));
}

#[test]
fn blend_with_empty_argument_stack_returns_error() {
    // `blend` pops `n` unconditionally. Without the emptiness check the pop
    // underflows the stack length, which panics rather than returning an error.
    let (result, path) = outline(&Cff2::new(vec![vec![operator::BLEND]]).build());

    assert_eq!(result.unwrap_err(), CFFError::InvalidArgumentsStackLength);
    assert_eq!(path, "");
}

#[test]
fn blend_with_empty_argument_stack_inside_subroutine_returns_error() {
    let mut font = Cff2::new(vec![call_subr(0, 1, operator::CALL_GLOBAL_SUBROUTINE)]);
    font.global_subrs = vec![vec![operator::BLEND]];

    let (result, path) = outline(&font.build());

    assert_eq!(result.unwrap_err(), CFFError::InvalidArgumentsStackLength);
    assert_eq!(path, "");
}

#[test]
fn blend_with_fewer_operands_than_n_blends_requires_returns_error() {
    // One region, so `2 blend` needs 2 * (1 + 1) = 4 operands. Only one is present.
    let mut char_string = Vec::new();
    char_string.extend_from_slice(&cs_int(100));
    char_string.extend_from_slice(&cs_int(2));
    char_string.push(operator::BLEND);

    let (result, _) = outline(&Cff2::new(vec![char_string]).build());

    assert_eq!(result.unwrap_err(), CFFError::InvalidArgumentsStackLength);
}

#[test]
fn blend_with_operand_count_larger_than_the_stack_returns_error() {
    // `n` is far beyond the 513-slot argument stack.
    let mut char_string = Vec::new();
    char_string.extend_from_slice(&cs_int(100));
    char_string.extend_from_slice(&cs_int(30000));
    char_string.push(operator::BLEND);

    let (result, _) = outline(&Cff2::new(vec![char_string]).build());

    assert_eq!(result.unwrap_err(), CFFError::InvalidArgumentsStackLength);
}

#[test]
fn endchar_operator_is_reserved_in_cff2() {
    let (result, _) = outline(&Cff2::new(vec![vec![operator::ENDCHAR]]).build());

    assert_eq!(result.unwrap_err(), CFFError::InvalidOperator);
}

#[test]
fn global_subroutine_call_budget_bounds_fanout_amplification() {
    // fanout=4, depth=8 nests at most 8 deep, below the STACK_LIMIT of 10, but
    // performs sum(4^L for L in 1..=8) = 87380 invocations, well past the 4096
    // budget. Without the budget the work grows as ~fanout^depth.
    assert!(87380 > MAX_SUBROUTINE_CALLS);

    let (result, _) = outline(&fanout_font(4, 8, false));

    assert_eq!(result.unwrap_err(), CFFError::SubroutineCallLimitReached);
}

#[test]
fn local_subroutine_call_budget_bounds_fanout_amplification() {
    let (result, _) = outline(&fanout_font(4, 8, true));

    assert_eq!(result.unwrap_err(), CFFError::SubroutineCallLimitReached);
}

#[test]
fn global_subroutine_chain_within_budget_outlines() {
    // The same depth without amplification is 8 invocations and must still work.
    let (result, path) = outline(&fanout_font(1, 8, false));

    assert_eq!(path, "M 100 0 L 150 50 Z ");
    assert_eq!(result.unwrap(), rect(100, 0, 150, 50));
}

#[test]
fn local_subroutine_chain_within_budget_outlines() {
    let (result, path) = outline(&fanout_font(1, 8, true));

    assert_eq!(path, "M 100 0 L 150 50 Z ");
    assert_eq!(result.unwrap(), rect(100, 0, 150, 50));
}

// Regression tests for https://github.com/harfbuzz/ttf-parser/issues/240. A CFF2 INDEX count
// is a raw u32, so `(count + 1) * offSize` reaches `Stream::read_bytes` orders of magnitude
// larger than the buffer. That used to trip a `debug_assert!` on `offset + len`, so a debug
// build aborted on input a release build rejected cleanly.
fn cff2_with_global_subr_index(count: u32, offset_size: u8) -> Vec<u8> {
    let mut top_dict = Vec::new();
    top_dict.extend_from_slice(&dict_int(100));
    top_dict.push(top_dict_operator::CHAR_STRINGS_OFFSET);
    top_dict.extend_from_slice(&dict_int(100));
    top_dict.extend_from_slice(&top_dict_operator::FONT_DICT_INDEX_OFFSET);

    let mut data = vec![2, 0, 5]; // majorVersion, minorVersion, headerSize
    data.extend_from_slice(&(top_dict.len() as u16).to_be_bytes());
    data.extend_from_slice(&top_dict);
    data.extend_from_slice(&count.to_be_bytes());
    data.push(offset_size);
    data
}

#[test]
fn index_count_whose_offset_array_exceeds_the_buffer_is_rejected() {
    // (0x3FFFFFFE + 1) * 4 == 0xFFFFFFFC, which fits in a u32 but not in the buffer.
    let data = cff2_with_global_subr_index(0x3FFFFFFE, 4);
    assert!(cff2::Table::parse(&data).is_none());
}

#[test]
fn index_count_whose_offset_array_overflows_a_u32_is_rejected() {
    let data = cff2_with_global_subr_index(u32::MAX - 1, 4);
    assert!(cff2::Table::parse(&data).is_none());
}
