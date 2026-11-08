//! DWARF inline instances: abstract origins, concrete ranges and nesting.
//!
//! The compiler records one [`DwarfInlineInstance`] per cloned callee body and
//! codegen emits zero-width begin/end labels around the instructions that still
//! belong to that instance.  This module pairs those labels after final layout,
//! checks the resulting tree, and writes it as data for the shared DWARF writer.
//! It never guesses an address from a source line: a range exists only when two
//! code labels prove it, and an instance whose body was completely optimized
//! away is omitted (the same observable choice Clang 23.1.5 makes; GCC keeps
//! zero-width DIEs, which this writer does not claim).

use super::*;

/// One proven `[start, end)` code range of an inline instance.
#[derive(Clone, Debug)]
pub(super) struct InlineRange {
    pub section_index: usize,
    pub start: u64,
    pub end: u64,
    pub start_label: String,
    pub end_label: String,
}

#[derive(Debug)]
struct InlineNode<'a> {
    spec: &'a crate::backend::elf::DwarfInlineInstance,
    parent: Option<usize>,
    ranges: Vec<InlineRange>,
    children: Vec<usize>,
    live: bool,
}

/// Validated inline tree for one unit, indexed once rather than rescanned per
/// function.  Parent ids must name an earlier instance in the same concrete
/// function, so the graph is acyclic by construction and iterative emission
/// cannot overflow the host stack on a deep inline chain.
#[derive(Debug)]
pub(super) struct InlineIndex<'a> {
    nodes: Vec<InlineNode<'a>>,
    roots_by_function: FxHashMap<&'a str, Vec<usize>>,
}

impl<'a> InlineIndex<'a> {
    pub fn new(
        unit: &'a DwarfUnitSpec,
        markers: &[DwarfInlineMarker],
        sections: &[DwarfSectionInfo<'_>],
    ) -> Result<Self, String> {
        let mut keys: FxHashMap<(&str, u32), usize> = FxHashMap::default();
        let mut nodes = Vec::with_capacity(unit.inline_instances.len());
        let mut roots_by_function: FxHashMap<&str, Vec<usize>> = FxHashMap::default();
        for (index, spec) in unit.inline_instances.iter().enumerate() {
            if spec.function.as_bytes().contains(&0) || spec.callee.as_bytes().contains(&0) {
                return Err("DWARF inline instance name contains a NUL byte".to_string());
            }
            let key = (spec.function.as_str(), spec.id);
            if keys.contains_key(&key) {
                return Err(format!(
                    "DWARF inline instance {} in {} is declared twice",
                    spec.id, spec.function
                ));
            }
            let parent = match spec.parent {
                None => None,
                Some(parent_id) => {
                    let parent =
                        *keys
                            .get(&(spec.function.as_str(), parent_id))
                            .ok_or_else(|| {
                                format!(
                                    "DWARF inline instance {} in {} names missing parent {}",
                                    spec.id, spec.function, parent_id
                                )
                            })?;
                    if parent >= index {
                        return Err("DWARF inline parent is not an earlier instance".to_string());
                    }
                    Some(parent)
                }
            };
            keys.insert(key, index);
            if parent.is_none() {
                roots_by_function
                    .entry(spec.function.as_str())
                    .or_default()
                    .push(index);
            }
            nodes.push(InlineNode {
                spec,
                parent,
                ranges: Vec::new(),
                children: Vec::new(),
                live: false,
            });
        }

        // Pair begin/end labels by (instance, serial).  A serial is minted once
        // per opening in codegen, so duplicates or a missing half are a layout
        // bug, not a range to silently drop.
        let mut pairs: FxHashMap<
            (usize, u32),
            (Option<&DwarfInlineMarker>, Option<&DwarfInlineMarker>),
        > = FxHashMap::default();
        for marker in markers {
            let index = *keys
                .get(&(marker.function.as_str(), marker.instance))
                .ok_or_else(|| {
                    format!(
                        "DWARF inline marker {} names unknown instance {} in {}",
                        marker.label, marker.instance, marker.function
                    )
                })?;
            let slot = pairs.entry((index, marker.serial)).or_default();
            let half = if marker.begin {
                &mut slot.0
            } else {
                &mut slot.1
            };
            if half.is_some() {
                return Err(format!(
                    "DWARF inline marker {} is duplicated",
                    marker.label
                ));
            }
            *half = Some(marker);
        }
        for ((index, serial), (begin, end)) in pairs {
            let begin = begin.ok_or_else(|| {
                format!("DWARF inline instance {index} serial {serial} has no begin label")
            })?;
            let end = end.ok_or_else(|| {
                format!("DWARF inline instance {index} serial {serial} has no end label")
            })?;
            if begin.section_index != end.section_index {
                return Err(format!(
                    "DWARF inline range {} crosses sections",
                    begin.label
                ));
            }
            let section = sections.get(begin.section_index).ok_or_else(|| {
                format!(
                    "DWARF inline marker {} names a missing section",
                    begin.label
                )
            })?;
            if !section.is_executable {
                return Err(format!(
                    "DWARF inline marker {} is in a non-code section",
                    begin.label
                ));
            }
            if end.offset < begin.offset || end.offset > section.size {
                return Err(format!(
                    "DWARF inline range {}..{} is outside its section",
                    begin.label, end.label
                ));
            }
            // A zero-width range carries no executable address.  Do not turn it
            // into a breakpoint claim; the instance disappears if every range
            // does, matching the absence of an out-of-line body.
            if end.offset > begin.offset {
                nodes[index].ranges.push(InlineRange {
                    section_index: begin.section_index,
                    start: begin.offset,
                    end: end.offset,
                    start_label: begin.label.clone(),
                    end_label: end.label.clone(),
                });
            }
        }

        for (index, node) in nodes.iter_mut().enumerate() {
            node.ranges.sort_by(|a, b| {
                (a.section_index, a.start, a.end).cmp(&(b.section_index, b.start, b.end))
            });
            for pair in node.ranges.windows(2) {
                if pair[0].section_index == pair[1].section_index && pair[0].end > pair[1].start {
                    return Err(format!(
                        "DWARF inline instance {index} has overlapping concrete ranges"
                    ));
                }
            }
            node.live = !node.ranges.is_empty();
        }
        // A parent with no instructions of its own still has the markers that
        // opened around its live child.  Propagate liveness so its abstract
        // origin and nesting are retained.
        for index in (0..nodes.len()).rev() {
            if nodes[index].live {
                if let Some(parent) = nodes[index].parent {
                    nodes[parent].live = true;
                    nodes[parent].children.push(index);
                }
            }
        }
        for node in &nodes {
            if node.live && node.ranges.is_empty() {
                return Err(format!(
                    "DWARF inline instance {} in {} has a live child but no concrete range",
                    node.spec.id, node.spec.function
                ));
            }
            if let Some(parent) = node.parent {
                for range in &node.ranges {
                    if !nodes[parent].ranges.iter().any(|outer| {
                        outer.section_index == range.section_index
                            && outer.start <= range.start
                            && range.end <= outer.end
                    }) {
                        return Err(format!(
                            "DWARF inline instance {} in {} escapes its parent's range",
                            node.spec.id, node.spec.function
                        ));
                    }
                }
            }
        }
        let range_keys: Vec<(usize, u64, u64)> = nodes
            .iter()
            .map(|node| {
                node.ranges
                    .first()
                    .map(|range| (range.section_index, range.start, range.end))
                    .unwrap_or((usize::MAX, u64::MAX, u64::MAX))
            })
            .collect();
        for roots in roots_by_function.values_mut() {
            roots.sort_by_key(|&index| range_keys[index]);
        }
        // Sort each child list once here, not per emission, and reject
        // overlapping siblings: two inline instances under the same parent
        // describe disjoint call sites, so an overlap is a layout/provenance
        // bug, not a nesting the debugger can represent.
        for node in nodes.iter_mut() {
            node.children.sort_by_key(|&index| range_keys[index]);
        }
        let check_siblings = |siblings: &[usize]| -> Result<(), String> {
            let mut ranges: Vec<(usize, u64, u64, usize)> = Vec::new();
            for &sibling in siblings {
                if !nodes[sibling].live {
                    continue;
                }
                for range in &nodes[sibling].ranges {
                    ranges.push((range.section_index, range.start, range.end, sibling));
                }
            }
            ranges.sort_unstable();
            for pair in ranges.windows(2) {
                let (section_a, _, end_a, node_a) = pair[0];
                let (section_b, start_b, _, node_b) = pair[1];
                if section_a == section_b && node_a != node_b && start_b < end_a {
                    return Err(format!(
                        "DWARF inline siblings {node_a} and {node_b} overlap at {start_b:#x}"
                    ));
                }
            }
            Ok(())
        };
        for roots in roots_by_function.values() {
            check_siblings(roots)?;
        }
        for node in &nodes {
            check_siblings(&node.children)?;
        }
        Ok(Self {
            nodes,
            roots_by_function,
        })
    }

    fn live_roots(&self, function: &str) -> Vec<usize> {
        self.roots_by_function
            .get(function)
            .map(|roots| {
                roots
                    .iter()
                    .copied()
                    .filter(|&index| self.nodes[index].live)
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Range-list section shared by the CU and its inline instances.  The CU keeps
/// its historical `start_length` encoding on non-relaxing delivery; inline
/// ranges use `start_end` with two label relocations so an assembler/linker that
/// deletes bytes between the endpoints cannot invalidate a folded length.
pub(super) struct RangeLists {
    config: DwarfConfig,
    address_size: usize,
    abs_addr: u32,
    data: Vec<u8>,
    relocs: Vec<ObjReloc>,
    header: bool,
}

impl RangeLists {
    pub fn for_code(
        config: DwarfConfig,
        code: &[&DwarfSectionInfo<'_>],
        address_size: usize,
        abs_addr: u32,
    ) -> Result<Self, String> {
        let mut lists = Self {
            config,
            address_size,
            abs_addr,
            data: Vec::new(),
            relocs: Vec::new(),
            header: false,
        };
        if code.len() < 2 {
            return Ok(lists);
        }
        lists.ensure_header();
        for section in code {
            if section.size == 0 {
                continue;
            }
            if config.version >= 5 {
                lists.data.push(DW_RLE_START_LENGTH);
                push_reloc(
                    &mut lists.data,
                    &mut lists.relocs,
                    abs_addr,
                    address_size,
                    section.name,
                    0,
                )?;
                write_uleb128(section.size, &mut lists.data);
            } else {
                push_reloc(
                    &mut lists.data,
                    &mut lists.relocs,
                    abs_addr,
                    address_size,
                    section.name,
                    0,
                )?;
                push_reloc(
                    &mut lists.data,
                    &mut lists.relocs,
                    abs_addr,
                    address_size,
                    section.name,
                    section.size,
                )?;
            }
        }
        lists.push_end_of_list();
        Ok(lists)
    }

    fn ensure_header(&mut self) {
        if self.config.version >= 5 && !self.header {
            self.data.extend_from_slice(&[0; 4]);
            self.data.extend_from_slice(&5u16.to_le_bytes());
            self.data.push(self.address_size as u8);
            self.data.push(0);
            self.data.extend_from_slice(&0u32.to_le_bytes());
            self.header = true;
        }
    }

    /// DWARF 5 `.debug_rnglists` ends a list with the single byte
    /// `DW_RLE_end_of_list`; DWARF 2..4 `.debug_ranges` ends it with a PAIR of
    /// zero addresses (`2 * address_size` bytes).  Emitting the v5 byte into a
    /// v2 section left a one-byte terminator, and addr2line could not resolve
    /// `-ffunction-sections -gdwarf-2` images (measured: dwarf-versions gate).
    fn push_end_of_list(&mut self) {
        if self.config.version >= 5 {
            self.data.push(DW_RLE_END_OF_LIST);
        } else {
            self.data
                .extend(std::iter::repeat_n(0u8, 2 * self.address_size));
        }
    }

    fn push_address(&mut self, symbol: &str) -> Result<(), String> {
        push_reloc(
            &mut self.data,
            &mut self.relocs,
            self.abs_addr,
            self.address_size,
            symbol,
            0,
        )
    }

    pub fn emit(&mut self, ranges: &[InlineRange]) -> Result<u32, String> {
        if ranges.is_empty() {
            return Err("DWARF inline range list is empty".to_string());
        }
        self.ensure_header();
        let offset = u32::try_from(self.data.len())
            .map_err(|_| "DWARF range list exceeds 32 bits".to_string())?;
        for range in ranges {
            if self.config.version >= 5 {
                self.data.push(DW_RLE_START_END);
            }
            self.push_address(&range.start_label)?;
            self.push_address(&range.end_label)?;
        }
        self.push_end_of_list();
        Ok(offset)
    }

    pub fn finish(mut self) -> Result<Option<ObjSection>, String> {
        if self.data.is_empty() {
            return Ok(None);
        }
        if self.config.version >= 5 {
            let length = u32::try_from(self.data.len() - 4)
                .map_err(|_| "DWARF range list exceeds 32 bits".to_string())?;
            self.data[..4].copy_from_slice(&length.to_le_bytes());
            Ok(Some(debug_section(
                ".debug_rnglists",
                self.data,
                self.relocs,
            )))
        } else {
            if self.data.len() > u32::MAX as usize {
                return Err("DWARF range list exceeds 32 bits".to_string());
            }
            Ok(Some(debug_section(".debug_ranges", self.data, self.relocs)))
        }
    }
}

/// The abstract instance of one callee, shared by every concrete inline DIE.
struct AbstractOrigin {
    offset: u32,
    parameters: Vec<u32>,
}

pub(super) struct AbstractOrigins<'a> {
    by_name: FxHashMap<&'a str, AbstractOrigin>,
}

pub(super) fn emit_abstracts<'a>(
    sink: &mut SubprogramSink<'_, '_, '_>,
    unit: &'a DwarfUnitSpec,
    index: &InlineIndex<'a>,
    abbrevs: &UnitAbbrevs,
    file_attr: &FxHashMap<&str, u64>,
) -> Result<AbstractOrigins<'a>, String> {
    let mut by_name: FxHashMap<&str, AbstractOrigin> = FxHashMap::default();
    for node in &index.nodes {
        if !node.live || by_name.contains_key(node.spec.callee.as_str()) {
            continue;
        }
        let spec = node.spec;
        let offset = u32::try_from(sink.info.len() - sink.unit_start)
            .map_err(|_| "DWARF abstract origin exceeds 32 bits".to_string())?;
        let parameters = unit.parameters_of(&spec.callee);
        let has_children = parameters.is_some();
        let mut attrs = vec![Attr {
            at: DW_AT_NAME,
            value: AttrValue::Text(spec.callee.clone()),
        }];
        if let Some(file) = spec
            .decl_file
            .as_deref()
            .and_then(|name| file_attr.get(name))
        {
            attrs.push(Attr {
                at: DW_AT_DECL_FILE,
                value: AttrValue::Udata(*file),
            });
            if spec.decl_line != 0 {
                attrs.push(Attr {
                    at: DW_AT_DECL_LINE,
                    value: AttrValue::Udata(u64::from(spec.decl_line)),
                });
                if spec.decl_column != 0 {
                    attrs.push(Attr {
                        at: DW_AT_DECL_COLUMN,
                        value: AttrValue::Udata(u64::from(spec.decl_column)),
                    });
                }
            }
        }
        // Only claim a prototype when sema recorded one; a K&R definition's
        // body parameters are not a prototype.  Missing metadata omits the
        // flag rather than lying.
        if unit.function_prototypes.get(&spec.callee).copied() == Some(true) {
            attrs.push(Attr {
                at: DW_AT_PROTOTYPED,
                value: AttrValue::FlagPresent,
            });
        }
        // `return_types` deliberately omits void returns (DWARF: no type) and
        // may lack a callee whose metadata was pruned.  Omit the attribute in
        // both cases; falling back to void invented a return type for every
        // non-void inline callee without metadata.
        if let Some(&ty) = unit.return_types.get(&spec.callee) {
            attrs.push(Attr {
                at: DW_AT_TYPE,
                value: AttrValue::TypeRef(ty),
            });
        }
        attrs.push(Attr {
            at: DW_AT_INLINE,
            value: AttrValue::Data1(if spec.declared_inline {
                DW_INL_DECLARED_INLINED
            } else {
                DW_INL_INLINED
            }),
        });
        let mut parameter_offsets = Vec::new();
        {
            let mut ctx = DieContext {
                info: sink.info,
                relocs: sink.relocs,
                strings: sink.strings,
                line_strings: sink.line_strings,
                config: sink.config,
                forms: sink.forms,
                abs32: sink.abs32,
                abs_addr: sink.abs_addr,
                address_size: sink.address_size,
                unit_start: sink.unit_start,
                writer: sink.dies,
            };
            ctx.write(DW_TAG_SUBPROGRAM, has_children, &attrs)?;
            if let Some(list) = parameters {
                for parameter in &list.parameters {
                    let param_offset = u32::try_from(ctx.info.len() - ctx.unit_start)
                        .map_err(|_| "DWARF abstract parameter exceeds 32 bits".to_string())?;
                    let code = if parameter.name.is_some() {
                        abbrevs.formal_parameter_named
                    } else {
                        abbrevs.formal_parameter_unnamed
                    };
                    write_uleb128(code, ctx.info);
                    if let Some(name) = parameter.name.as_deref() {
                        if name.as_bytes().contains(&0) {
                            return Err(format!(
                                "DWARF abstract parameter name contains a NUL byte: {name:?}"
                            ));
                        }
                        write_string_attribute(
                            name,
                            ctx.forms.string,
                            ctx.info,
                            ctx.relocs,
                            ctx.abs32,
                            ctx.strings,
                            ctx.line_strings,
                        )?;
                    }
                    ctx.writer.patches.push((ctx.info.len(), parameter.ty));
                    ctx.info.extend_from_slice(&[0; 4]);
                    parameter_offsets.push(param_offset);
                }
                if list.variadic {
                    write_uleb128(abbrevs.unspecified_parameters, ctx.info);
                }
            }
            if has_children {
                ctx.info.push(0);
            }
        }
        // GCC 16.2 publishes a fully-inlined static helper's abstract origin in
        // `.debug_pubnames`; the concrete copy may not exist.  Publish the same
        // name from our own tree so `-gpubnames` resolves the inline callee.
        if sink.config.pubnames {
            sink.dies
                .pubnames
                .push((u64::from(offset), spec.callee.clone()));
        }
        by_name.insert(
            spec.callee.as_str(),
            AbstractOrigin {
                offset,
                parameters: parameter_offsets,
            },
        );
    }
    Ok(AbstractOrigins { by_name })
}

pub(super) fn has_live_roots(index: &InlineIndex<'_>, function: &str) -> bool {
    index
        .roots_by_function
        .get(function)
        .is_some_and(|roots| roots.iter().any(|&root| index.nodes[root].live))
}

/// Emit the inline forest under one concrete subprogram.  The explicit event
/// stack mirrors the lexical-block writer and bounds host stack use by the
/// output tree, not by the C call chain that produced it.
pub(super) fn emit_children(
    sink: &mut SubprogramSink<'_, '_, '_>,
    function: &DwarfFunction<'_>,
    index: &InlineIndex<'_>,
    abstracts: &AbstractOrigins<'_>,
    file_attr: &FxHashMap<&str, u64>,
    ranges: &mut RangeLists,
) -> Result<(), String> {
    let roots = index.live_roots(function.name);
    if roots.is_empty() {
        return Ok(());
    }
    let function_end = function
        .offset
        .checked_add(function.size)
        .ok_or_else(|| "DWARF function range overflows".to_string())?;
    for &root in &roots {
        for range in &index.nodes[root].ranges {
            if range.section_index != function.section_index
                || range.start < function.offset
                || range.end > function_end
            {
                return Err(format!(
                    "DWARF inline instance {} escapes function {}",
                    index.nodes[root].spec.id, function.name
                ));
            }
        }
    }

    enum Event {
        Enter(usize),
        Exit,
    }
    let mut stack: Vec<Event> = roots.into_iter().rev().map(Event::Enter).collect();
    while let Some(event) = stack.pop() {
        match event {
            Event::Exit => sink.info.push(0),
            Event::Enter(node_index) => {
                let node = &index.nodes[node_index];
                let abstract_origin = abstracts
                    .by_name
                    .get(node.spec.callee.as_str())
                    .ok_or_else(|| {
                        format!(
                            "DWARF inline callee {} has no abstract origin",
                            node.spec.callee
                        )
                    })?;
                let children = &node.children;
                let has_children = !children.is_empty() || !abstract_origin.parameters.is_empty();
                let mut attrs = vec![Attr {
                    at: DW_AT_ABSTRACT_ORIGIN,
                    value: AttrValue::DieRef(abstract_origin.offset),
                }];
                if node.ranges.len() == 1 {
                    let range = &node.ranges[0];
                    attrs.push(Attr {
                        at: DW_AT_LOW_PC,
                        value: AttrValue::Address(range.start_label.clone()),
                    });
                    // An address-form high_pc is legal in every supported
                    // version and remains correct if a later assembler relaxes
                    // the bytes between the two labels.
                    attrs.push(Attr {
                        at: DW_AT_HIGH_PC,
                        value: AttrValue::Address(range.end_label.clone()),
                    });
                } else {
                    let offset = ranges.emit(&node.ranges)?;
                    attrs.push(Attr {
                        at: DW_AT_RANGES,
                        value: AttrValue::RangeOffset(offset),
                    });
                }
                if let Some(file) = node.spec.call_file.as_deref() {
                    let file = *file_attr.get(file).ok_or_else(|| {
                        format!("DWARF inline call file {file:?} is absent from the line table")
                    })?;
                    attrs.push(Attr {
                        at: DW_AT_CALL_FILE,
                        value: AttrValue::Udata(file),
                    });
                    if node.spec.call_line != 0 {
                        attrs.push(Attr {
                            at: DW_AT_CALL_LINE,
                            value: AttrValue::Udata(u64::from(node.spec.call_line)),
                        });
                        if node.spec.call_column != 0 {
                            attrs.push(Attr {
                                at: DW_AT_CALL_COLUMN,
                                value: AttrValue::Udata(u64::from(node.spec.call_column)),
                            });
                        }
                    }
                }
                {
                    let mut ctx = DieContext {
                        info: sink.info,
                        relocs: sink.relocs,
                        strings: sink.strings,
                        line_strings: sink.line_strings,
                        config: sink.config,
                        forms: sink.forms,
                        abs32: sink.abs32,
                        abs_addr: sink.abs_addr,
                        address_size: sink.address_size,
                        unit_start: sink.unit_start,
                        writer: sink.dies,
                    };
                    ctx.write(DW_TAG_INLINED_SUBROUTINE, has_children, &attrs)?;
                    for &parameter in &abstract_origin.parameters {
                        ctx.write(
                            DW_TAG_FORMAL_PARAMETER,
                            false,
                            &[Attr {
                                at: DW_AT_ABSTRACT_ORIGIN,
                                value: AttrValue::DieRef(parameter),
                            }],
                        )?;
                    }
                }
                if has_children {
                    stack.push(Event::Exit);
                    for &child in children.iter().rev() {
                        stack.push(Event::Enter(child));
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::elf::{DwarfInlineInstance, DwarfInlineMarker, DwarfSectionInfo};

    fn section() -> DwarfSectionInfo<'static> {
        DwarfSectionInfo {
            name: ".text",
            size: 4096,
            is_executable: true,
        }
    }

    fn instance(id: u32, callee: &str, parent: Option<u32>) -> DwarfInlineInstance {
        DwarfInlineInstance {
            function: "main".to_string(),
            id,
            callee: callee.to_string(),
            parent,
            call_file: Some("a.c".to_string()),
            call_line: 10 + id,
            call_column: 3,
            decl_file: Some("a.c".to_string()),
            decl_line: 1 + id,
            decl_column: 1,
            declared_inline: true,
        }
    }

    fn marker(instance: u32, serial: u32, begin: bool, offset: u64) -> DwarfInlineMarker {
        DwarfInlineMarker {
            label: format!(
                ".Llccc_inline_{}_{}_{}",
                if begin { 'b' } else { 'e' },
                instance,
                serial
            ),
            section_index: 0,
            offset,
            function: "main".to_string(),
            instance,
            serial,
            begin,
        }
    }

    fn unit(instances: Vec<DwarfInlineInstance>) -> DwarfUnitSpec {
        DwarfUnitSpec {
            inline_instances: instances,
            ..Default::default()
        }
    }

    #[test]
    fn nested_instances_validate_and_keep_parent_live() {
        let unit = unit(vec![
            instance(0, "middle", None),
            instance(1, "leaf", Some(0)),
        ]);
        let markers = vec![
            marker(0, 0, true, 16),
            marker(1, 0, true, 32),
            marker(1, 0, false, 48),
            marker(0, 0, false, 64),
        ];
        let index = InlineIndex::new(&unit, &markers, &[section()]).unwrap();
        assert!(has_live_roots(&index, "main"));
        assert_eq!(index.nodes[0].children, vec![1]);
        assert!(index.nodes[0].live && index.nodes[1].live);
    }

    #[test]
    fn duplicate_instance_id_is_rejected() {
        let unit = unit(vec![instance(0, "a", None), instance(0, "b", None)]);
        let err = InlineIndex::new(&unit, &[], &[section()]).unwrap_err();
        assert!(err.contains("declared twice"), "{err}");
    }

    #[test]
    fn missing_parent_is_rejected() {
        let unit = unit(vec![instance(1, "leaf", Some(0))]);
        let err = InlineIndex::new(&unit, &[], &[section()]).unwrap_err();
        assert!(err.contains("missing parent"), "{err}");
    }

    #[test]
    fn duplicate_marker_half_is_rejected() {
        let unit = unit(vec![instance(0, "a", None)]);
        let markers = vec![marker(0, 0, true, 16), marker(0, 0, true, 32)];
        let err = InlineIndex::new(&unit, &markers, &[section()]).unwrap_err();
        assert!(err.contains("duplicated"), "{err}");
    }

    #[test]
    fn missing_marker_half_is_rejected() {
        let unit = unit(vec![instance(0, "a", None)]);
        let markers = vec![marker(0, 0, true, 16)];
        let err = InlineIndex::new(&unit, &markers, &[section()]).unwrap_err();
        assert!(err.contains("no end label"), "{err}");
    }

    #[test]
    fn backwards_range_is_rejected() {
        let unit = unit(vec![instance(0, "a", None)]);
        let markers = vec![marker(0, 0, true, 64), marker(0, 0, false, 16)];
        let err = InlineIndex::new(&unit, &markers, &[section()]).unwrap_err();
        assert!(err.contains("outside its section"), "{err}");
    }

    #[test]
    fn non_code_section_is_rejected() {
        let unit = unit(vec![instance(0, "a", None)]);
        let markers = vec![marker(0, 0, true, 16), marker(0, 0, false, 32)];
        let data = DwarfSectionInfo {
            name: ".data",
            size: 4096,
            is_executable: false,
        };
        let err = InlineIndex::new(&unit, &markers, &[data]).unwrap_err();
        assert!(err.contains("non-code section"), "{err}");
    }

    #[test]
    fn child_escaping_parent_is_rejected() {
        let unit = unit(vec![
            instance(0, "middle", None),
            instance(1, "leaf", Some(0)),
        ]);
        let markers = vec![
            marker(0, 0, true, 16),
            marker(0, 0, false, 32),
            marker(1, 0, true, 48),
            marker(1, 0, false, 64),
        ];
        let err = InlineIndex::new(&unit, &markers, &[section()]).unwrap_err();
        assert!(err.contains("escapes its parent"), "{err}");
    }

    #[test]
    fn overlapping_siblings_are_rejected() {
        let unit = unit(vec![instance(0, "a", None), instance(1, "b", None)]);
        let markers = vec![
            marker(0, 0, true, 16),
            marker(0, 0, false, 48),
            marker(1, 0, true, 32),
            marker(1, 0, false, 64),
        ];
        let err = InlineIndex::new(&unit, &markers, &[section()]).unwrap_err();
        assert!(err.contains("siblings"), "{err}");
    }

    #[test]
    fn zero_width_instance_is_not_live() {
        let unit = unit(vec![instance(0, "a", None)]);
        let markers = vec![marker(0, 0, true, 16), marker(0, 0, false, 16)];
        let index = InlineIndex::new(&unit, &markers, &[section()]).unwrap();
        assert!(!has_live_roots(&index, "main"));
        assert!(!index.nodes[0].live);
    }

    #[test]
    fn range_list_terminator_width_follows_the_dialect() {
        let sections = [
            DwarfSectionInfo {
                name: ".text.a",
                size: 16,
                is_executable: true,
            },
            DwarfSectionInfo {
                name: ".text.b",
                size: 24,
                is_executable: true,
            },
        ];
        let code: Vec<&DwarfSectionInfo<'_>> = sections.iter().collect();
        for version in 2..=5 {
            let config = DwarfConfig::for_c(version, None);
            let mut lists = RangeLists::for_code(config, &code, 8, 1).unwrap();
            let range = InlineRange {
                section_index: 0,
                start: 0,
                end: 16,
                start_label: ".Llccc_inline_b_0_0_main".to_string(),
                end_label: ".Llccc_inline_e_0_0_main".to_string(),
            };
            lists.emit(&[range]).unwrap();
            let section = lists.finish().unwrap().unwrap();
            let width = if version >= 5 { 1 } else { 16 };
            assert!(
                section.data.ends_with(&vec![0u8; width]),
                "v{version} terminator width"
            );
            if version < 5 {
                assert_eq!(section.name, ".debug_ranges");
            } else {
                assert_eq!(section.name, ".debug_rnglists");
            }
        }
    }

    #[test]
    fn deep_inline_chain_does_not_recurse() {
        const DEPTH: u32 = 512;
        let instances: Vec<_> = (0..DEPTH)
            .map(|i| instance(i, &format!("f{i}"), if i == 0 { None } else { Some(i - 1) }))
            .collect();
        let unit = unit(instances);
        let mut markers = Vec::new();
        for i in 0..DEPTH {
            markers.push(marker(i, 0, true, u64::from(i)));
            markers.push(marker(i, 0, false, u64::from(2 * DEPTH - i)));
        }
        let index = InlineIndex::new(&unit, &markers, &[section()]).unwrap();
        assert!(has_live_roots(&index, "main"));
        assert!(index.nodes.iter().all(|node| node.live));
    }
}
