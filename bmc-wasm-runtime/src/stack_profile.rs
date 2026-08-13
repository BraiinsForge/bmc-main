// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

//! Post-link shadow-stack instrumentation for capture fixture profiling.

use anyhow::{Context as _, Result, anyhow, bail};
use wasm_encoder::reencode::{self, Reencode};
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, ExportKind, ExportSection, Function, FunctionSection,
    GlobalSection, GlobalType, Instruction, Module, TypeSection, ValType,
};
use wasmparser::{Operator, Parser, Payload, TypeRef};

pub const STACK_HIGH_WATER_EXPORT: &str = "__bmc_stack_high_water";

#[derive(Debug)]
struct ModuleShape {
    type_count: u32,
    updater_function: u32,
    updater_type: u32,
    stack_pointer_global: u32,
    stack_high_water_global: u32,
    stack_origin: i32,
}

/// Inject an exported counter that records the greatest shadow-stack use.
pub fn instrument(wasm: &[u8], expected_origin: i32) -> Result<Vec<u8>> {
    let shape = inspect_module(wasm)?;
    if shape.stack_origin != expected_origin {
        bail!(
            "wasm stack origin is {}, expected {expected_origin}",
            shape.stack_origin
        );
    }
    let mut module = Module::new();
    let mut reencoder = StackProfiler { shape };
    reencoder
        .parse_core_module(&mut module, Parser::new(0), wasm)
        .map_err(|error| anyhow!("failed to instrument wasm module: {error}"))?;
    Ok(module.finish())
}

fn inspect_module(wasm: &[u8]) -> Result<ModuleShape> {
    let mut type_count = 0_u32;
    let mut imported_functions = 0_u32;
    let mut defined_functions = 0_u32;
    let mut imported_globals = 0_u32;
    let mut defined_globals = 0_u32;
    let mut stack_origin = None;
    let mut has_type_section = false;
    let mut has_function_section = false;
    let mut has_export_section = false;
    let mut has_code_section = false;

    for payload in Parser::new(0).parse_all(wasm) {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "only module-shape sections affect stack instrumentation"
        )]
        match payload? {
            Payload::TypeSection(section) => {
                has_type_section = true;
                for ty in section.into_iter_err_on_gc_types() {
                    ty?;
                    type_count += 1;
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    match import?.ty {
                        TypeRef::Func(_) | TypeRef::FuncExact(_) => imported_functions += 1,
                        TypeRef::Global(_) => imported_globals += 1,
                        TypeRef::Table(_) | TypeRef::Memory(_) | TypeRef::Tag(_) => {}
                    }
                }
            }
            Payload::FunctionSection(section) => {
                has_function_section = true;
                defined_functions = section.count();
            }
            Payload::GlobalSection(section) => {
                defined_globals = section.count();
                let global = section
                    .into_iter()
                    .next()
                    .context("wasm module has no __stack_pointer global")??;
                if global.ty.content_type != wasmparser::ValType::I32 || !global.ty.mutable {
                    bail!("wasm module's first global is not a mutable i32 __stack_pointer");
                }
                let mut operators = global.init_expr.get_operators_reader();
                stack_origin = match (operators.read()?, operators.read()?) {
                    (Operator::I32Const { value }, Operator::End) => Some(value),
                    _ => bail!("wasm module's __stack_pointer is not initialized by i32.const"),
                };
            }
            Payload::ExportSection(section) => {
                has_export_section = true;
                for export in section {
                    if export?.name == STACK_HIGH_WATER_EXPORT {
                        bail!("wasm module already exports {STACK_HIGH_WATER_EXPORT}");
                    }
                }
            }
            Payload::CodeSectionStart { .. } => has_code_section = true,
            _ => {}
        }
    }

    if !(has_type_section && has_function_section && has_export_section && has_code_section) {
        bail!("wasm module is missing a section required for stack profiling");
    }

    Ok(ModuleShape {
        type_count,
        updater_function: imported_functions + defined_functions,
        updater_type: type_count,
        stack_pointer_global: imported_globals,
        stack_high_water_global: imported_globals + defined_globals,
        stack_origin: stack_origin.context("wasm module has no global section")?,
    })
}

#[derive(Debug)]
struct StackProfiler {
    shape: ModuleShape,
}

impl Reencode for StackProfiler {
    type Error = anyhow::Error;

    fn parse_type_section(
        &mut self,
        types: &mut TypeSection,
        section: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_type_section(self, types, section)?;
        debug_assert_eq!(types.len(), self.shape.type_count);
        types.ty().function([], []);
        Ok(())
    }

    fn parse_function_section(
        &mut self,
        functions: &mut FunctionSection,
        section: wasmparser::FunctionSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_function_section(self, functions, section)?;
        functions.function(self.shape.updater_type);
        Ok(())
    }

    fn parse_global_section(
        &mut self,
        globals: &mut GlobalSection,
        section: wasmparser::GlobalSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_global_section(self, globals, section)?;
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        Ok(())
    }

    fn parse_export_section(
        &mut self,
        exports: &mut ExportSection,
        section: wasmparser::ExportSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        reencode::utils::parse_export_section(self, exports, section)?;
        exports.export(
            STACK_HIGH_WATER_EXPORT,
            ExportKind::Global,
            self.shape.stack_high_water_global,
        );
        Ok(())
    }

    fn parse_code_section(
        &mut self,
        code: &mut CodeSection,
        section: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        for body in section {
            self.parse_function_body(code, body?)?;
        }
        code.function(&self.high_water_helper());
        Ok(())
    }

    fn parse_function_body(
        &mut self,
        code: &mut CodeSection,
        body: wasmparser::FunctionBody<'_>,
    ) -> Result<(), reencode::Error<Self::Error>> {
        let mut function = self.new_function_with_parsed_locals(&body)?;
        let mut operators = body.get_operators_reader()?;
        while !operators.eof() {
            let operator = operators.read()?;
            let updates_stack_pointer = matches!(
                operator,
                Operator::GlobalSet { global_index }
                    if global_index == self.shape.stack_pointer_global
            );
            function.instruction(&self.instruction(operator)?);
            if updates_stack_pointer {
                function.instruction(&Instruction::Call(self.shape.updater_function));
            }
        }
        code.function(&function);
        Ok(())
    }
}

impl StackProfiler {
    fn high_water_helper(&self) -> Function {
        let mut function = Function::new([]);
        function
            .instruction(&Instruction::I32Const(self.shape.stack_origin))
            .instruction(&Instruction::GlobalGet(self.shape.stack_pointer_global))
            .instruction(&Instruction::I32Sub)
            .instruction(&Instruction::GlobalGet(self.shape.stack_high_water_global))
            .instruction(&Instruction::I32GtU)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(self.shape.stack_origin))
            .instruction(&Instruction::GlobalGet(self.shape.stack_pointer_global))
            .instruction(&Instruction::I32Sub)
            .instruction(&Instruction::GlobalSet(self.shape.stack_high_water_global))
            .instruction(&Instruction::End)
            .instruction(&Instruction::End);
        function
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_lowest_stack_pointer_even_after_it_recovers() {
        let wasm = wat::parse_str(
            r#"(module
                (global $stack (mut i32) (i32.const 65536))
                (func (export "exercise")
                    i32.const 64000
                    global.set $stack
                    i32.const 65000
                    global.set $stack))"#,
        )
        .expect("BUG: test module must compile");
        let instrumented = instrument(&wasm, 65_536).expect("instrumentation must succeed");

        let engine = wasmi::Engine::default();
        let module = wasmi::Module::new(&engine, &instrumented[..])
            .expect("instrumented module must validate");
        let mut store = wasmi::Store::new(&engine, ());
        let instance = wasmi::Linker::new(&engine)
            .instantiate_and_start(&mut store, &module)
            .expect("instrumented module must instantiate");
        instance
            .get_typed_func::<(), ()>(&store, "exercise")
            .expect("test export must exist")
            .call(&mut store, ())
            .expect("test export must run");

        let high_water = instance
            .get_global(&store, STACK_HIGH_WATER_EXPORT)
            .expect("instrumentation must export its measurement")
            .get(&store)
            .i32()
            .expect("measurement must be an i32");
        assert_eq!(high_water, 1_536);
    }

    #[test]
    fn preserves_indices_when_functions_and_globals_are_imported() {
        let wasm = wat::parse_str(
            r#"(module
                (import "host" "sentinel" (global i32))
                (import "host" "noop" (func))
                (global $stack (mut i32) (i32.const 65536))
                (func (export "exercise")
                    i32.const 64000
                    global.set $stack))"#,
        )
        .expect("BUG: test module must compile");
        let instrumented = instrument(&wasm, 65_536).expect("instrumentation must succeed");

        wasmi::Module::new(&wasmi::Engine::default(), &instrumented[..])
            .expect("instrumented module with imports must validate");
    }

    #[test]
    fn rejects_a_stack_origin_that_disagrees_with_the_policy() {
        let wasm = wat::parse_str(
            r#"(module
                (global (mut i32) (i32.const 65536))
                (func (export "exercise")))"#,
        )
        .expect("BUG: test module must compile");

        assert!(instrument(&wasm, 1_048_576).is_err());
    }
}
