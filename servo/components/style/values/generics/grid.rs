/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Generic types for the handling of
//! [grids](https://drafts.csswg.org/css-grid/).

use crate::parser::{Parse, ParserContext};
use crate::values::specified;
use crate::values::{CSSFloat, CustomIdent};
use crate::{One, Zero};
use cssparser::Parser;
use std::fmt::{self, Write};
use std::{cmp, usize};
use style_traits::{CssWriter, ParseError, StyleParseErrorKind, ToCss};

/// These are the limits that we choose to clamp grid line numbers to.
/// http://drafts.csswg.org/css-grid/#overlarge-grids
/// line_num is clamped to this range at parse time.
pub const MIN_GRID_LINE: i32 = -10000;
/// See above.
pub const MAX_GRID_LINE: i32 = 10000;

/// A `<grid-line>` type.
///
/// <https://drafts.csswg.org/css-grid/#typedef-grid-row-start-grid-line>
#[derive(
    Clone,
    Debug,
    Default,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C)]
pub struct GenericGridLine<Integer> {
    /// A custom identifier for named lines, or the empty atom otherwise.
    ///
    /// <https://drafts.csswg.org/css-grid/#grid-placement-slot>
    pub ident: CustomIdent,
    /// Denotes the nth grid line from grid item's placement.
    ///
    /// This is clamped by MIN_GRID_LINE and MAX_GRID_LINE.
    ///
    /// NOTE(emilio): If we ever allow animating these we need to either do
    /// something more complicated for the clamping, or do this clamping at
    /// used-value time.
    pub line_num: Integer,
    /// Flag to check whether it's a `span` keyword.
    pub is_span: bool,
}

pub use self::GenericGridLine as GridLine;

impl<Integer> GridLine<Integer>
where
    Integer: PartialEq + Zero,
{
    /// The `auto` value.
    pub fn auto() -> Self {
        Self {
            is_span: false,
            line_num: Zero::zero(),
            ident: CustomIdent(atom!("")),
        }
    }

    /// Check whether this `<grid-line>` represents an `auto` value.
    pub fn is_auto(&self) -> bool {
        self.ident.0 == atom!("") && self.line_num.is_zero() && !self.is_span
    }

    /// Check whether this `<grid-line>` represents a `<custom-ident>` value.
    pub fn is_ident_only(&self) -> bool {
        self.ident.0 != atom!("") && self.line_num.is_zero() && !self.is_span
    }

    /// Check if `self` makes `other` omittable according to the rules at:
    /// https://drafts.csswg.org/css-grid/#propdef-grid-column
    /// https://drafts.csswg.org/css-grid/#propdef-grid-area
    pub fn can_omit(&self, other: &Self) -> bool {
        if self.is_ident_only() {
            self == other
        } else {
            other.is_auto()
        }
    }
}

impl<Integer> ToCss for GridLine<Integer>
where
    Integer: ToCss + PartialEq + Zero + One,
{
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        // 1. `auto`
        if self.is_auto() {
            return dest.write_str("auto");
        }

        // 2. `<custom-ident>`
        if self.is_ident_only() {
            return self.ident.to_css(dest);
        }

        // 3. `[ span && [ <integer [1,∞]> || <custom-ident> ] ]`
        let has_ident = self.ident.0 != atom!("");
        if self.is_span {
            dest.write_str("span")?;
            debug_assert!(!self.line_num.is_zero() || has_ident);

            // We omit `line_num` if
            // 1. we don't specify it, or
            // 2. it is the default value, i.e. 1.0, and the ident is specified.
            // https://drafts.csswg.org/css-grid/#grid-placement-span-int
            if !self.line_num.is_zero() && !(self.line_num.is_one() && has_ident) {
                dest.write_char(' ')?;
                self.line_num.to_css(dest)?;
            }

            if has_ident {
                dest.write_char(' ')?;
                self.ident.to_css(dest)?;
            }
            return Ok(());
        }

        // 4. `[ <integer [-∞,-1]> | <integer [1,∞]> ] && <custom-ident>? ]`
        debug_assert!(!self.line_num.is_zero());
        self.line_num.to_css(dest)?;
        if has_ident {
            dest.write_char(' ')?;
            self.ident.to_css(dest)?;
        }
        Ok(())
    }
}

impl Parse for GridLine<specified::Integer> {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        let mut grid_line = Self::auto();
        if input.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
            return Ok(grid_line);
        }

        // <custom-ident> | [ <integer> && <custom-ident>? ] | [ span && [ <integer> || <custom-ident> ] ]
        // This <grid-line> horror is simply,
        // [ span? && [ <custom-ident> || <integer> ] ]
        // And, for some magical reason, "span" should be the first or last value and not in-between.
        let mut val_before_span = false;

        for _ in 0..3 {
            // Maximum possible entities for <grid-line>
            let location = input.current_source_location();
            if input.try_parse(|i| i.expect_ident_matching("span")).is_ok() {
                if grid_line.is_span {
                    return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                }

                if !grid_line.line_num.is_zero() || grid_line.ident.0 != atom!("") {
                    val_before_span = true;
                }

                grid_line.is_span = true;
            } else if let Ok(i) = input.try_parse(|i| specified::Integer::parse(context, i)) {
                // FIXME(emilio): Probably shouldn't reject if it's calc()...
                let value = i.value();
                if value == 0 || val_before_span || !grid_line.line_num.is_zero() {
                    return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                }

                grid_line.line_num = specified::Integer::new(cmp::max(
                    MIN_GRID_LINE,
                    cmp::min(value, MAX_GRID_LINE),
                ));
            } else if let Ok(name) = input.try_parse(|i| CustomIdent::parse(i, &["auto"])) {
                if val_before_span || grid_line.ident.0 != atom!("") {
                    return Err(location.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                }
                // NOTE(emilio): `span` is consumed above, so we only need to
                // reject `auto`.
                grid_line.ident = name;
            } else {
                break;
            }
        }

        if grid_line.is_auto() {
            return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
        }

        if grid_line.is_span {
            if !grid_line.line_num.is_zero() {
                if grid_line.line_num.value() <= 0 {
                    // disallow negative integers for grid spans
                    return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
                }
            } else if grid_line.ident.0 == atom!("") {
                // integer could be omitted
                return Err(input.new_custom_error(StyleParseErrorKind::UnspecifiedError));
            }
        }

        Ok(grid_line)
    }
}

/// A track breadth for explicit grid track sizing. It's generic solely to
/// avoid re-implementing it for the computed type.
///
/// <https://drafts.csswg.org/css-grid/#typedef-track-breadth>
#[derive(
    Animate,
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C, u8)]
pub enum GenericTrackBreadth<L> {
    /// The generic type is almost always a non-negative `<length-percentage>`
    Breadth(L),
    /// A flex fraction specified in `fr` units.
    #[css(dimension)]
    Fr(CSSFloat),
    /// `auto`
    Auto,
    /// `min-content`
    MinContent,
    /// `max-content`
    MaxContent,
}

pub use self::GenericTrackBreadth as TrackBreadth;

impl<L> TrackBreadth<L> {
    /// Check whether this is a `<fixed-breadth>` (i.e., it only has `<length-percentage>`)
    ///
    /// <https://drafts.csswg.org/css-grid/#typedef-fixed-breadth>
    #[inline]
    pub fn is_fixed(&self) -> bool {
        matches!(*self, TrackBreadth::Breadth(..))
    }
}

/// A `<track-size>` type for explicit grid track sizing. Like `<track-breadth>`, this is
/// generic only to avoid code bloat. It only takes `<length-percentage>`
///
/// <https://drafts.csswg.org/css-grid/#typedef-track-size>
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C, u8)]
pub enum GenericTrackSize<L> {
    /// A flexible `<track-breadth>`
    Breadth(GenericTrackBreadth<L>),
    /// A `minmax` function for a range over an inflexible `<track-breadth>`
    /// and a flexible `<track-breadth>`
    ///
    /// <https://drafts.csswg.org/css-grid/#valdef-grid-template-columns-minmax>
    #[css(function)]
    Minmax(GenericTrackBreadth<L>, GenericTrackBreadth<L>),
    /// A `fit-content` function.
    ///
    /// This stores a TrackBreadth<L> for convenience, but it can only be a
    /// LengthPercentage.
    ///
    /// <https://drafts.csswg.org/css-grid/#valdef-grid-template-columns-fit-content>
    #[css(function)]
    FitContent(GenericTrackBreadth<L>),
}

pub use self::GenericTrackSize as TrackSize;

impl<L> TrackSize<L> {
    /// The initial value.
    const INITIAL_VALUE: Self = TrackSize::Breadth(TrackBreadth::Auto);

    /// Returns the initial value.
    pub const fn initial_value() -> Self {
        Self::INITIAL_VALUE
    }

    /// Returns true if `self` is the initial value.
    pub fn is_initial(&self) -> bool {
        matches!(*self, TrackSize::Breadth(TrackBreadth::Auto)) // FIXME: can't use Self::INITIAL_VALUE here yet: https://github.com/rust-lang/rust/issues/66585
    }

    /// Check whether this is a `<fixed-size>`
    ///
    /// <https://drafts.csswg.org/css-grid/#typedef-fixed-size>
    pub fn is_fixed(&self) -> bool {
        match *self {
            TrackSize::Breadth(ref breadth) => breadth.is_fixed(),
            // For minmax function, it could be either
            // minmax(<fixed-breadth>, <track-breadth>) or minmax(<inflexible-breadth>, <fixed-breadth>),
            // and since both variants are a subset of minmax(<inflexible-breadth>, <track-breadth>), we only
            // need to make sure that they're fixed. So, we don't have to modify the parsing function.
            TrackSize::Minmax(ref breadth_1, ref breadth_2) => {
                if breadth_1.is_fixed() {
                    return true; // the second value is always a <track-breadth>
                }

                match *breadth_1 {
                    TrackBreadth::Fr(_) => false, // should be <inflexible-breadth> at this point
                    _ => breadth_2.is_fixed(),
                }
            },
            TrackSize::FitContent(_) => false,
        }
    }
}

impl<L> Default for TrackSize<L> {
    fn default() -> Self {
        Self::initial_value()
    }
}

impl<L: ToCss> ToCss for TrackSize<L> {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        match *self {
            TrackSize::Breadth(ref breadth) => breadth.to_css(dest),
            TrackSize::Minmax(ref min, ref max) => {
                // According to gecko minmax(auto, <flex>) is equivalent to <flex>,
                // and both are serialized as <flex>.
                if let TrackBreadth::Auto = *min {
                    if let TrackBreadth::Fr(_) = *max {
                        return max.to_css(dest);
                    }
                }

                dest.write_str("minmax(")?;
                min.to_css(dest)?;
                dest.write_str(", ")?;
                max.to_css(dest)?;
                dest.write_char(')')
            },
            TrackSize::FitContent(ref lp) => {
                dest.write_str("fit-content(")?;
                lp.to_css(dest)?;
                dest.write_char(')')
            },
        }
    }
}

/// A `<track-size>+`.
/// We use the empty slice as `auto`, and always parse `auto` as an empty slice.
/// This means it's impossible to have a slice containing only one auto item.
#[derive(
    Clone,
    Debug,
    Default,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
)]
#[repr(transparent)]
pub struct GenericImplicitGridTracks<T>(
    #[css(if_empty = "auto", iterable)] pub crate::OwnedSlice<T>,
);

pub use self::GenericImplicitGridTracks as ImplicitGridTracks;

impl<T: fmt::Debug + Default + PartialEq> ImplicitGridTracks<T> {
    /// Returns true if current value is same as its initial value (i.e. auto).
    pub fn is_initial(&self) -> bool {
        debug_assert_ne!(
            *self,
            ImplicitGridTracks(crate::OwnedSlice::from(vec![Default::default()]))
        );
        self.0.is_empty()
    }
}

/// Helper function for serializing identifiers with a prefix and suffix, used
/// for serializing <line-names> (in grid).
pub fn concat_serialize_idents<W>(
    prefix: &str,
    suffix: &str,
    slice: &[CustomIdent],
    sep: &str,
    dest: &mut CssWriter<W>,
) -> fmt::Result
where
    W: Write,
{
    if let Some((ref first, rest)) = slice.split_first() {
        dest.write_str(prefix)?;
        first.to_css(dest)?;
        for thing in rest {
            dest.write_str(sep)?;
            thing.to_css(dest)?;
        }

        dest.write_str(suffix)?;
    }

    Ok(())
}

/// The initial argument of the `repeat` function.
///
/// <https://drafts.csswg.org/css-grid/#typedef-track-repeat>
#[derive(
    Clone,
    Copy,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C, u8)]
pub enum RepeatCount<Integer> {
    /// A positive integer. This is allowed only for `<track-repeat>` and `<fixed-repeat>`
    Number(Integer),
    /// An `<auto-fill>` keyword allowed only for `<auto-repeat>`
    AutoFill,
    /// An `<auto-fit>` keyword allowed only for `<auto-repeat>`
    AutoFit,
}

impl Parse for RepeatCount<specified::Integer> {
    fn parse<'i, 't>(
        context: &ParserContext,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self, ParseError<'i>> {
        if let Ok(mut i) = input.try_parse(|i| specified::Integer::parse_positive(context, i)) {
            if i.value() > MAX_GRID_LINE {
                i = specified::Integer::new(MAX_GRID_LINE);
            }
            return Ok(RepeatCount::Number(i));
        }
        try_match_ident_ignore_ascii_case! { input,
            "auto-fill" => Ok(RepeatCount::AutoFill),
            "auto-fit" => Ok(RepeatCount::AutoFit),
        }
    }
}

/// The structure containing `<line-names>` and `<track-size>` values.
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[css(function = "repeat")]
#[repr(C)]
pub struct GenericTrackRepeat<L, I> {
    /// The number of times for the value to be repeated (could also be `auto-fit` or `auto-fill`)
    pub count: RepeatCount<I>,
    /// `<line-names>` accompanying `<track_size>` values.
    ///
    /// If there's no `<line-names>`, then it's represented by an empty vector.
    /// For N `<track-size>` values, there will be N+1 `<line-names>`, and so this vector's
    /// length is always one value more than that of the `<track-size>`.
    pub line_names: crate::OwnedSlice<crate::OwnedSlice<CustomIdent>>,
    /// `<track-size>` values.
    pub track_sizes: crate::OwnedSlice<GenericTrackSize<L>>,
}

pub use self::GenericTrackRepeat as TrackRepeat;

impl<L: ToCss, I: ToCss> ToCss for TrackRepeat<L, I> {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        dest.write_str("repeat(")?;
        self.count.to_css(dest)?;
        dest.write_str(", ")?;

        let mut line_names_iter = self.line_names.iter();
        for (i, (ref size, ref names)) in self
            .track_sizes
            .iter()
            .zip(&mut line_names_iter)
            .enumerate()
        {
            if i > 0 {
                dest.write_char(' ')?;
            }

            concat_serialize_idents("[", "] ", names, " ", dest)?;
            size.to_css(dest)?;
        }

        if let Some(line_names_last) = line_names_iter.next() {
            concat_serialize_idents(" [", "]", line_names_last, " ", dest)?;
        }

        dest.write_char(')')?;

        Ok(())
    }
}

/// Track list values. Can be <track-size> or <track-repeat>
#[derive(
    Animate,
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C, u8)]
pub enum GenericTrackListValue<LengthPercentage, Integer> {
    /// A <track-size> value.
    TrackSize(#[animation(field_bound)] GenericTrackSize<LengthPercentage>),
    /// A <track-repeat> value.
    TrackRepeat(#[animation(field_bound)] GenericTrackRepeat<LengthPercentage, Integer>),
}

pub use self::GenericTrackListValue as TrackListValue;

impl<L, I> TrackListValue<L, I> {
    // FIXME: can't use TrackSize::initial_value() here b/c rustc error "is not yet stable as a const fn"
    const INITIAL_VALUE: Self = TrackListValue::TrackSize(TrackSize::Breadth(TrackBreadth::Auto));

    fn is_repeat(&self) -> bool {
        matches!(*self, TrackListValue::TrackRepeat(..))
    }

    /// Returns true if `self` is the initial value.
    pub fn is_initial(&self) -> bool {
        matches!(
            *self,
            TrackListValue::TrackSize(TrackSize::Breadth(TrackBreadth::Auto))
        ) // FIXME: can't use Self::INITIAL_VALUE here yet: https://github.com/rust-lang/rust/issues/66585
    }
}

impl<L, I> Default for TrackListValue<L, I> {
    #[inline]
    fn default() -> Self {
        Self::INITIAL_VALUE
    }
}

/// A grid `<track-list>` type.
///
/// <https://drafts.csswg.org/css-grid/#typedef-track-list>
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C)]
pub struct GenericTrackList<LengthPercentage, Integer> {
    /// The index in `values` where our `<auto-repeat>` value is, if in bounds.
    #[css(skip)]
    pub auto_repeat_index: usize,
    /// A vector of `<track-size> | <track-repeat>` values.
    pub values: crate::OwnedSlice<GenericTrackListValue<LengthPercentage, Integer>>,
    /// `<line-names>` accompanying `<track-size> | <track-repeat>` values.
    ///
    /// If there's no `<line-names>`, then it's represented by an empty vector.
    /// For N values, there will be N+1 `<line-names>`, and so this vector's
    /// length is always one value more than that of the `<track-size>`.
    pub line_names: crate::OwnedSlice<crate::OwnedSlice<CustomIdent>>,
}

pub use self::GenericTrackList as TrackList;

impl<L, I> TrackList<L, I> {
    /// Whether this track list is an explicit track list (that is, doesn't have
    /// any repeat values).
    pub fn is_explicit(&self) -> bool {
        !self.values.iter().any(|v| v.is_repeat())
    }

    /// Whether this track list has an `<auto-repeat>` value.
    pub fn has_auto_repeat(&self) -> bool {
        self.auto_repeat_index < self.values.len()
    }
}

impl<L: ToCss, I: ToCss> ToCss for TrackList<L, I> {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        let mut values_iter = self.values.iter().peekable();
        let mut line_names_iter = self.line_names.iter().peekable();

        for idx in 0.. {
            let names = line_names_iter.next().unwrap(); // This should exist!
            concat_serialize_idents("[", "]", names, " ", dest)?;

            match values_iter.next() {
                Some(value) => {
                    if !names.is_empty() {
                        dest.write_char(' ')?;
                    }

                    value.to_css(dest)?;
                },
                None => break,
            }

            if values_iter.peek().is_some() ||
                line_names_iter.peek().map_or(false, |v| !v.is_empty()) ||
                (idx + 1 == self.auto_repeat_index)
            {
                dest.write_char(' ')?;
            }
        }

        Ok(())
    }
}

/// The `<name-repeat>` for subgrids.
///
/// <name-repeat> = repeat( [ <integer [1,∞]> | auto-fill ], <line-names>+)
///
/// https://drafts.csswg.org/css-grid/#typedef-name-repeat
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C)]
pub struct GenericNameRepeat<I> {
    /// The number of times for the value to be repeated (could also be `auto-fill`).
    /// Note: `RepeatCount` accepts `auto-fit`, so we should reject it after parsing it.
    pub count: RepeatCount<I>,
    /// This represents `<line-names>+`. The length of the outer vector is at least one.
    pub line_names: crate::OwnedSlice<crate::OwnedSlice<CustomIdent>>,
}

pub use self::GenericNameRepeat as NameRepeat;

impl<I: ToCss> ToCss for NameRepeat<I> {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        dest.write_str("repeat(")?;
        self.count.to_css(dest)?;
        dest.write_char(',')?;

        for ref names in self.line_names.iter() {
            if names.is_empty() {
                // Note: concat_serialize_idents() skip the empty list so we have to handle it
                // manually for NameRepeat.
                dest.write_str(" []")?;
            } else {
                concat_serialize_idents(" [", "]", names, " ", dest)?;
            }
        }

        dest.write_char(')')
    }
}

impl<I> NameRepeat<I> {
    /// Returns true if it is auto-fill.
    #[inline]
    pub fn is_auto_fill(&self) -> bool {
        matches!(self.count, RepeatCount::AutoFill)
    }
}

/// A single value for `<line-names>` or `<name-repeat>`.
#[derive(
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C, u8)]
pub enum GenericLineNameListValue<I> {
    /// `<line-names>`.
    LineNames(crate::OwnedSlice<CustomIdent>),
    /// `<name-repeat>`.
    Repeat(GenericNameRepeat<I>),
}

pub use self::GenericLineNameListValue as LineNameListValue;

impl<I: ToCss> ToCss for LineNameListValue<I> {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        match *self {
            Self::Repeat(ref r) => r.to_css(dest),
            Self::LineNames(ref names) => {
                dest.write_char('[')?;

                if let Some((ref first, rest)) = names.split_first() {
                    first.to_css(dest)?;
                    for name in rest {
                        dest.write_char(' ')?;
                        name.to_css(dest)?;
                    }
                }

                dest.write_char(']')
            },
        }
    }
}

/// The `<line-name-list>` for subgrids.
///
/// <line-name-list> = [ <line-names> | <name-repeat> ]+
/// <name-repeat> = repeat( [ <integer [1,∞]> | auto-fill ], <line-names>+)
///
/// https://drafts.csswg.org/css-grid/#typedef-line-name-list
#[derive(
    Clone,
    Debug,
    Default,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToResolvedValue,
    ToShmem,
)]
#[repr(C)]
pub struct GenericLineNameList<I>{
    /// The pre-computed length of line_names, without the length of repeat(auto-fill, ...).
    // We precomputed this at parsing time, so we can avoid an extra loop when expanding
    // repeat(auto-fill).
    pub expanded_line_names_length: usize,
    /// The line name list.
    pub line_names: crate::OwnedSlice<GenericLineNameListValue<I>>,
}

pub use self::GenericLineNameList as LineNameList;

impl<I: ToCss> ToCss for LineNameList<I> {
    fn to_css<W>(&self, dest: &mut CssWriter<W>) -> fmt::Result
    where
        W: Write,
    {
        dest.write_str("subgrid")?;

        for value in self.line_names.iter() {
            dest.write_char(' ')?;
            value.to_css(dest)?;
        }

        Ok(())
    }
}

/// Variants for `<grid-template-rows> | <grid-template-columns>`
#[derive(
    Animate,
    Clone,
    Debug,
    MallocSizeOf,
    PartialEq,
    SpecifiedValueInfo,
    ToComputedValue,
    ToCss,
    ToResolvedValue,
    ToShmem,
)]
#[value_info(other_values = "subgrid")]
#[repr(C, u8)]
pub enum GenericGridTemplateComponent<L, I> {
    /// `none` value.
    None,
    /// The grid `<track-list>`
    TrackList(
        #[animation(field_bound)]
        #[compute(field_bound)]
        #[resolve(field_bound)]
        #[shmem(field_bound)]
        Box<GenericTrackList<L, I>>,
    ),
    /// A `subgrid <line-name-list>?`
    /// TODO: Support animations for this after subgrid is addressed in [grid-2] spec.
    #[animation(error)]
    Subgrid(Box<GenericLineNameList<I>>),
    /// `masonry` value.
    /// https://github.com/w3c/csswg-drafts/issues/4650
    Masonry,
}

pub use self::GenericGridTemplateComponent as GridTemplateComponent;

impl<L, I> GridTemplateComponent<L, I> {
    /// The initial value.
    const INITIAL_VALUE: Self = Self::None;

    /// Returns length of the <track-list>s <track-size>
    pub fn track_list_len(&self) -> usize {
        match *self {
            GridTemplateComponent::TrackList(ref tracklist) => tracklist.values.len(),
            _ => 0,
        }
    }

    /// Returns true if `self` is the initial value.
    pub fn is_initial(&self) -> bool {
        matches!(*self, Self::None) // FIXME: can't use Self::INITIAL_VALUE here yet: https://github.com/rust-lang/rust/issues/66585
    }
}

impl<L, I> Default for GridTemplateComponent<L, I> {
    #[inline]
    fn default() -> Self {
        Self::INITIAL_VALUE
    }
}
