//! A [Font Variations Table](
//! https://docs.microsoft.com/en-us/typography/opentype/spec/fvar) implementation.

use core::num::NonZeroU16;

use crate::parser::{f32_bound, Fixed, FromData, LazyArray16, Offset, Offset16, Stream};
use crate::{NormalizedCoordinate, Tag};

/// A [variation axis](https://docs.microsoft.com/en-us/typography/opentype/spec/fvar#variationaxisrecord).
#[repr(C)]
#[allow(missing_docs)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct VariationAxis {
    pub tag: Tag,
    pub min_value: f32,
    pub def_value: f32,
    pub max_value: f32,
    /// An axis name in the `name` table.
    pub name_id: u16,
    pub hidden: bool,
}

impl FromData for VariationAxis {
    const SIZE: usize = 20;

    fn parse(data: &[u8]) -> Option<Self> {
        let mut s = Stream::new(data);
        let tag = s.read::<Tag>()?;
        let min_value = s.read::<Fixed>()?;
        let def_value = s.read::<Fixed>()?;
        let max_value = s.read::<Fixed>()?;
        let flags = s.read::<u16>()?;
        let name_id = s.read::<u16>()?;

        Some(VariationAxis {
            tag,
            min_value: def_value.0.min(min_value.0),
            def_value: def_value.0,
            max_value: def_value.0.max(max_value.0),
            name_id,
            hidden: (flags >> 3) & 1 == 1,
        })
    }
}

impl VariationAxis {
    /// Returns a normalized variation coordinate for this axis.
    pub(crate) fn normalized_value(&self, mut v: f32) -> NormalizedCoordinate {
        // Based on
        // https://docs.microsoft.com/en-us/typography/opentype/spec/avar#overview

        v = f32_bound(self.min_value, v, self.max_value);
        if v == self.def_value {
            v = 0.0;
        } else if v < self.def_value {
            v = (v - self.def_value) / (self.def_value - self.min_value);
        } else {
            v = (v - self.def_value) / (self.max_value - self.def_value);
        }

        NormalizedCoordinate::from(v)
    }
}

/// An [instance record](https://docs.microsoft.com/en-us/typography/opentype/spec/fvar#instancerecord).
#[derive(Clone, Copy, Debug)]
pub struct Instance<'a> {
    /// The name ID for entries in the 'name' table that provide subfamily names for this instance.
    pub subfamily_name_id: u16,
    /// Reserved for future use — set to 0.
    pub flags: u16,
    /// The coordinate array for this instance (length = axisCount).
    pub coordinates: LazyArray16<'a, Fixed>,
    /// The name ID for entries in the 'name' table that provide PostScript names for this instance.
    pub post_script_name_id: Option<u16>,
}

impl<'a> Instance<'a> {
    /// Returns an iterator over the coordinate values as `f32`.
    ///
    /// This is a convenience method that converts the internal `Fixed` representation
    /// to floating-point values.
    #[inline]
    pub fn coordinates_f32(&self) -> impl Iterator<Item = f32> + 'a {
        self.coordinates.into_iter().map(|fixed| fixed.0)
    }

    #[inline]
    fn parse_from_record(
        record: &'a [u8],
        axis_count: u16,
        has_post_script_name_id: bool,
    ) -> Option<Self> {
        let mut s = Stream::new(record);
        let subfamily_name_id = s.read::<u16>()?;
        let flags = s.read::<u16>()?;
        let coordinates = s.read_array16::<Fixed>(axis_count)?;
        let post_script_name_id = if has_post_script_name_id {
            Some(s.read::<u16>()?)
        } else {
            None
        };
        Some(Self {
            subfamily_name_id,
            flags,
            coordinates,
            post_script_name_id,
        })
    }
}

/// A view over the `InstanceRecord` array.
#[derive(Clone, Copy, Debug)]
pub struct Instances<'a> {
    data: &'a [u8],
    record_len: u16,
    axis_count: u16,
    count: u16,
}

/// An iterator over [instance records](Instance).
#[derive(Clone, Copy, Debug)]
pub struct InstancesIter<'a> {
    instances: Instances<'a>,
    index: u16,
}

impl<'a> Iterator for InstancesIter<'a> {
    type Item = Instance<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.instances.count {
            let instance = self.instances.get(self.index)?;
            self.index += 1;
            Some(instance)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.instances.count - self.index) as usize;
        (remaining, Some(remaining))
    }
}

impl<'a> ExactSizeIterator for InstancesIter<'a> {
    fn len(&self) -> usize {
        (self.instances.count - self.index) as usize
    }
}

impl<'a> IntoIterator for Instances<'a> {
    type Item = Instance<'a>;
    type IntoIter = InstancesIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        InstancesIter {
            instances: self,
            index: 0,
        }
    }
}

impl<'a> Instances<'a> {
    /// Returns an iterator over the instance records.
    ///
    /// # Examples
    ///
    /// ```
    /// # use ttf_parser::fvar::Table;
    /// # let data = &[/* font data */];
    /// # if let Some(table) = Table::parse(data) {
    /// for instance in table.instances.iter() {
    ///     println!("Subfamily name ID: {}", instance.subfamily_name_id);
    /// }
    /// # }
    /// ```
    #[inline]
    pub fn iter(&self) -> InstancesIter<'a> {
        (*self).into_iter()
    }

    #[inline]
    fn new(data: &'a [u8], record_len: u16, axis_count: u16, count: u16) -> Self {
        Self {
            data,
            record_len,
            axis_count,
            count,
        }
    }

    /// Total number of instance records.
    #[inline]
    pub fn len(&self) -> u16 {
        self.count
    }

    /// Returns true when there are no instance records.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Returns `true` when the `postScriptNameID` field is present in records.
    #[inline]
    pub fn has_post_script_name_id(&self) -> bool {
        // The base size is 4 bytes (subfamilyNameID + flags) + 4 bytes per axis coordinate.
        // If record_len is at least base + 2, the optional postScriptNameID field is present.
        let axis_count = usize::from(self.axis_count);
        let base = 4 + 4 * axis_count;
        usize::from(self.record_len) >= base + 2
    }

    /// Returns the instance at the given index.
    ///
    /// Returns `None` if the index is out of bounds.
    pub fn get(&self, index: u16) -> Option<Instance<'a>> {
        if index >= self.count {
            return None;
        }
        let len = usize::from(self.record_len);
        let start = usize::from(index) * len;
        let end = start + len;
        let record = self.data.get(start..end)?;
        Instance::parse_from_record(record, self.axis_count, self.has_post_script_name_id())
    }
}

/// A [Font Variations Table](
/// https://docs.microsoft.com/en-us/typography/opentype/spec/fvar).
#[derive(Clone, Copy, Debug)]
pub struct Table<'a> {
    /// A list of variation axes.
    pub axes: LazyArray16<'a, VariationAxis>,
    /// A list of instance records.
    pub instances: Instances<'a>,
}

impl<'a> Table<'a> {
    /// Parses a table from raw data.
    pub fn parse(data: &'a [u8]) -> Option<Self> {
        let mut s = Stream::new(data);
        let version = s.read::<u32>()?;
        if version != 0x00010000 {
            return None;
        }

        let axes_array_offset = s.read::<Offset16>()?;
        s.skip::<u16>(); // reserved
        let axis_count = s.read::<u16>()?;
        let axis_size = s.read::<u16>()?;
        let instance_count = s.read::<u16>()?;
        let instance_size = s.read::<u16>()?;

        if axis_size as usize != VariationAxis::SIZE {
            return None;
        }

        // 'If axisCount is zero, then the font is not functional as a variable font,
        // and must be treated as a non-variable font;
        // any variation-specific tables or data is ignored.'
        let axis_count = NonZeroU16::new(axis_count)?;

        let mut s = Stream::new_at(data, axes_array_offset.to_usize())?;
        let axes = s.read_array16::<VariationAxis>(axis_count.get())?;

        // Instance records follow the axes array immediately.
        let instances_offset = axes_array_offset
            .to_usize()
            .checked_add(usize::from(axis_count.get()) * VariationAxis::SIZE)?;

        // Validate instance record size: must be base or base + 2 (for psNameID).
        let base = 4usize.checked_add(4usize.checked_mul(usize::from(axis_count.get()))?)?;
        let inst_size = usize::from(instance_size);
        if inst_size < base {
            return None;
        }

        let total_instances_len =
            usize::from(instance_count).checked_mul(usize::from(instance_size))?;
        let mut inst_stream = Stream::new_at(data, instances_offset)?;
        let inst_data = inst_stream.read_bytes(total_instances_len)?;
        let instances = Instances::new(inst_data, instance_size, axis_count.get(), instance_count);

        Some(Table { axes, instances })
    }
}
