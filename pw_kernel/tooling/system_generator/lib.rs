// Copyright 2025 The Pigweed Authors
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not
// use this file except in compliance with the License. You may obtain a copy of
// the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
// License for the specific language governing permissions and limitations under
// the License.

use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use minijinja::{Environment, State};
use serde::Serialize;
use serde::de::DeserializeOwned;

pub mod mpu_validation;
pub mod system_config;

use mpu_validation::{MpuValidationMode, validate_pmsav7_layout, format_pmsav7_analysis, suggest_pmsav7_friendly_layout};
use system_config::SystemConfig;

#[derive(Debug, Parser)]
pub struct Cli {
    #[command(flatten)]
    common_args: CommonArgs,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args, Debug)]
pub struct CommonArgs {
    #[arg(long, required(true))]
    config: PathBuf,
    #[arg(long, required(true))]
    output: PathBuf,
    #[arg(
        long("template"),
        value_name = "NAME=PATH",
        value_parser = parse_template,
        action = clap::ArgAction::Append
    )]
    templates: Vec<(String, PathBuf)>,
}

fn parse_template(s: &str) -> Result<(String, PathBuf), String> {
    s.split_once('=')
        .map(|(name, path)| (name.to_string(), path.into()))
        .ok_or_else(|| format!("invalid template format: '{s}'. Expected NAME=PATH."))
}

#[derive(Subcommand, Debug)]
enum Command {
    RenderTargetTemplate,
    RenderAppTemplate(AppLinkerScriptArgs),
}

#[derive(Args, Debug)]
pub struct AppLinkerScriptArgs {
    #[arg(long, required = true)]
    pub app_name: String,
}

/// MPU architecture type for validation purposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MpuArchitecture {
    /// ARMv7-M PMSAv7 (power-of-2 regions with subregion disable)
    Pmsav7,
    /// ARMv8-M PMSAv8 (arbitrary regions with 32-byte granularity)
    Pmsav8,
    /// No MPU validation needed
    None,
}

pub trait ArchConfigInterface {
    fn get_arch_crate_name(&self) -> &'static str;
    fn get_start_fn_address(&self, flash_start_address: u64) -> u64;
    /// Returns the MPU architecture for this configuration.
    fn get_mpu_architecture(&self) -> MpuArchitecture;
}

pub fn parse_config<A: ArchConfigInterface + DeserializeOwned>(
    cli: &Cli,
) -> Result<system_config::SystemConfig<A>> {
    let json5_str =
        fs::read_to_string(&cli.common_args.config).context("Failed to read config file")?;
    let mut config: SystemConfig<A> =
        serde_json5::from_str(&json5_str).context("Failed to parse config file")?;
    config.calculate_and_validate()?;
    Ok(config)
}

const FLASH_ALIGNMENT: u64 = 4;
const RAM_ALIGNMENT: u64 = 8;

impl ArchConfigInterface for system_config::Armv8MConfig {
    fn get_arch_crate_name(&self) -> &'static str {
        "arch_arm_cortex_m"
    }

    fn get_start_fn_address(&self, flash_start_address: u64) -> u64 {
        // On Armv8M, the +1 is to denote thumb mode.
        flash_start_address + 1
    }

    fn get_mpu_architecture(&self) -> MpuArchitecture {
        MpuArchitecture::Pmsav8
    }
}

impl ArchConfigInterface for system_config::Armv7MConfig {
    fn get_arch_crate_name(&self) -> &'static str {
        "arch_arm_cortex_m"
    }

    fn get_start_fn_address(&self, flash_start_address: u64) -> u64 {
        // On Armv7M, the +1 is to denote thumb mode.
        flash_start_address + 1
    }

    fn get_mpu_architecture(&self) -> MpuArchitecture {
        MpuArchitecture::Pmsav7
    }
}

impl ArchConfigInterface for system_config::RiscVConfig {
    fn get_arch_crate_name(&self) -> &'static str {
        "arch_riscv"
    }

    fn get_start_fn_address(&self, flash_start_address: u64) -> u64 {
        flash_start_address
    }

    fn get_mpu_architecture(&self) -> MpuArchitecture {
        // RISC-V PMP validation not yet implemented
        MpuArchitecture::None
    }
}

pub struct SystemGenerator<'a, A: ArchConfigInterface> {
    cli: Cli,
    config: system_config::SystemConfig<A>,
    env: Environment<'a>,
}

impl<'a, A: ArchConfigInterface + Serialize> SystemGenerator<'a, A> {
    pub fn new(cli: Cli, config: system_config::SystemConfig<A>) -> Result<Self> {
        let mut instance = Self {
            cli,
            config,
            env: Environment::new(),
        };

        instance.env.add_filter("hex", hex);
        for (name, path) in instance.cli.common_args.templates.clone() {
            let template = fs::read_to_string(path)?;
            instance.env.add_template_owned(name, template)?;
        }

        instance.populate_addresses();
        instance.populate_interrupt_table()?;
        instance.validate_mpu_layout()?;

        Ok(instance)
    }

    pub fn generate(&mut self) -> Result<()> {
        let out_str = match &self.cli.command {
            Command::RenderTargetTemplate => self.render_system()?,
            Command::RenderAppTemplate(args) => self.render_app_linker_script(&args.app_name)?,
        };

        let mut file = File::create(&self.cli.common_args.output)?;
        file.write_all(out_str.as_bytes())
            .context("Failed to write output")
    }

    fn render_system(&self) -> Result<String> {
        let template = self.env.get_template("system")?;
        match template.render(&self.config) {
            Ok(str) => Ok(str),
            Err(e) => Err(anyhow!(e)),
        }
    }

    fn render_app_linker_script(&self, app_name: &String) -> Result<String> {
        let template = self.env.get_template("app")?;
        let app = self
            .config
            .apps
            .get(app_name)
            .ok_or_else(|| anyhow!("Unable to find app \"{app_name}\" in system manifest"))?;
        match template.render(app) {
            Ok(str) => Ok(str),
            Err(e) => Err(anyhow!(e)),
        }
    }

    #[must_use]
    fn align(value: u64, alignment: u64) -> u64 {
        debug_assert!(alignment.is_power_of_two());
        (value + alignment - 1) & !(alignment - 1)
    }

    fn populate_addresses(&mut self) {
        // Stack the apps after the kernel in flash and ram.
        // TODO: davidroth - remove the requirement of setting the size of
        // flash, and instead calculate it based on code size.
        let mut next_flash_start_address =
            self.config.kernel.flash_start_address + self.config.kernel.flash_size_bytes;
        next_flash_start_address = Self::align(next_flash_start_address, FLASH_ALIGNMENT);
        let mut next_ram_start_address =
            self.config.kernel.ram_start_address + self.config.kernel.ram_size_bytes;
        next_ram_start_address = Self::align(next_ram_start_address, RAM_ALIGNMENT);

        self.config.arch_crate_name = self.config.arch.get_arch_crate_name();

        for app in self.config.apps.values_mut() {
            app.flash_start_address = next_flash_start_address;
            next_flash_start_address = Self::align(
                app.flash_start_address + app.flash_size_bytes,
                FLASH_ALIGNMENT,
            );

            app.ram_start_address = next_ram_start_address;
            next_ram_start_address =
                Self::align(app.ram_start_address + app.ram_size_bytes, RAM_ALIGNMENT);

            app.start_fn_address = self
                .config
                .arch
                .get_start_fn_address(app.flash_start_address);

            app.initial_sp = app.ram_start_address + app.ram_size_bytes;
        }
    }

    fn populate_interrupt_table(&mut self) -> Result<()> {
        if self.config.kernel.interrupt_table.is_none() {
            return Ok(());
        }

        let interrupt_table = self.config.kernel.interrupt_table.as_mut().unwrap();

        // Calculate the size of the interrupt table, which is the highest handled IRQ + 1
        interrupt_table.table_size = interrupt_table
            .table
            .keys()
            .max()
            .map(|max_irq| (max_irq.parse::<usize>().unwrap()) + 1)
            .unwrap_or(0);

        Ok(())
    }

    /// Validate memory layout for MPU compatibility.
    ///
    /// This checks that the memory layout is compatible with the target's MPU
    /// architecture. For PMSAv7 (ARMv7-M), this includes checking that MPU
    /// subregions don't overlap with kernel memory.
    fn validate_mpu_layout(&self) -> Result<()> {
        let mpu_arch = self.config.arch.get_mpu_architecture();
        let validation_mode = self.config.kernel.mpu_validation;

        // Skip validation if mode is permissive or no MPU
        if validation_mode == MpuValidationMode::Permissive || mpu_arch == MpuArchitecture::None {
            return Ok(());
        }

        match mpu_arch {
            MpuArchitecture::Pmsav7 => self.validate_pmsav7(),
            MpuArchitecture::Pmsav8 => {
                // PMSAv8 validation is less strict - just basic overlap checks
                // TODO: Implement PMSAv8 region count validation
                Ok(())
            }
            MpuArchitecture::None => Ok(()),
        }
    }

    /// Validate memory layout for PMSAv7 compatibility.
    fn validate_pmsav7(&self) -> Result<()> {
        let validation_mode = self.config.kernel.mpu_validation;

        // Build app list for validation
        let apps: Vec<_> = self
            .config
            .apps
            .iter()
            .map(|(name, app)| {
                (
                    name.clone(),
                    app.flash_start_address,
                    app.flash_start_address + app.flash_size_bytes,
                    app.ram_start_address,
                    app.ram_start_address + app.ram_size_bytes,
                )
            })
            .collect();

        let kernel_flash_end =
            self.config.kernel.flash_start_address + self.config.kernel.flash_size_bytes;
        let kernel_ram_end =
            self.config.kernel.ram_start_address + self.config.kernel.ram_size_bytes;

        let issues = validate_pmsav7_layout(
            self.config.kernel.flash_start_address,
            kernel_flash_end,
            self.config.kernel.ram_start_address,
            kernel_ram_end,
            &apps,
        );

        if issues.is_empty() {
            return Ok(());
        }

        // Print detailed analysis for each problematic region
        eprintln!("\n======================================================================");
        eprintln!("MPU MEMORY LAYOUT VALIDATION");
        eprintln!("======================================================================");

        // Print memory layout
        eprintln!("\nMemory Layout:");
        eprintln!(
            "  Kernel Flash: {:#010x} - {:#010x} ({}KB)",
            self.config.kernel.flash_start_address,
            kernel_flash_end,
            self.config.kernel.flash_size_bytes / 1024
        );
        eprintln!(
            "  Kernel RAM:   {:#010x} - {:#010x} ({}KB)",
            self.config.kernel.ram_start_address,
            kernel_ram_end,
            self.config.kernel.ram_size_bytes / 1024
        );
        for (name, app) in &self.config.apps {
            let flash_end = app.flash_start_address + app.flash_size_bytes;
            let ram_end = app.ram_start_address + app.ram_size_bytes;
            eprintln!(
                "  App '{}' Flash: {:#010x} - {:#010x} ({}KB)",
                name,
                app.flash_start_address,
                flash_end,
                app.flash_size_bytes / 1024
            );
            eprintln!(
                "  App '{}' RAM:   {:#010x} - {:#010x} ({}KB)",
                name,
                app.ram_start_address,
                ram_end,
                app.ram_size_bytes / 1024
            );
        }

        // Print PMSAv7 analysis for affected regions
        eprintln!("\n----------------------------------------------------------------------");
        eprintln!("PMSAv7 SUBREGION ANALYSIS");
        eprintln!("----------------------------------------------------------------------");

        for (name, app) in &self.config.apps {
            let flash_end = app.flash_start_address + app.flash_size_bytes;
            eprintln!(
                "\n{}",
                format_pmsav7_analysis(
                    &format!("App '{}' Flash", name),
                    app.flash_start_address,
                    flash_end
                )
            );
        }

        // Print issues
        let has_errors = issues.iter().any(|i| i.code == "MPU001" || i.code == "MPU003");

        eprintln!("\n----------------------------------------------------------------------");
        eprintln!("VALIDATION ISSUES");
        eprintln!("----------------------------------------------------------------------");

        for issue in &issues {
            let severity = if issue.code == "MPU001" || issue.code == "MPU003" {
                "error"
            } else {
                "warning"
            };
            eprintln!("\n{}: {}", severity, issue);
        }

        // Print suggested layout
        if has_errors {
            let apps_for_suggestion: Vec<_> = self
                .config
                .apps
                .iter()
                .map(|(name, app)| (name.clone(), app.flash_size_bytes, app.ram_size_bytes))
                .collect();

            eprintln!("\n----------------------------------------------------------------------");
            eprintln!(
                "{}",
                suggest_pmsav7_friendly_layout(
                    0x420, // Typical vector table size
                    self.config.kernel.flash_size_bytes,
                    self.config.kernel.ram_size_bytes,
                    &apps_for_suggestion,
                )
            );
        }

        eprintln!("======================================================================\n");

        // Fail the build if strict mode and there are errors
        if validation_mode == MpuValidationMode::Strict && has_errors {
            return Err(anyhow!(
                "MPU validation failed: {} issue(s) found. \
                 Use 'mpu_validation: \"warn\"' in kernel config to allow builds with warnings.",
                issues.len()
            ));
        }

        Ok(())
    }
}

// Custom filters
#[must_use]
pub fn hex(_state: &State, value: usize) -> String {
    format!("{value:#x}")
}
