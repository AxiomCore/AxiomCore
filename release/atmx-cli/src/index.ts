#!/usr/bin/env node
import { Command } from "commander";
import * as fs from "fs-extra";
import * as path from "path";
import * as toml from "@iarna/toml";
import { MultiIR } from "./types";
import { generateModels } from "./generators/model-generator";
import { generateSdk } from "./generators/sdk-generator";
import { normalizeIr } from "./generators/utils";

const program = new Command();

program
  .name("atmx")
  .description("Generate TypeScript SDK from AxiomDeps.toml")
  .version("0.2.0");

program
  .command("generate")
  .requiredOption("-c, --config <path>", "Path to AxiomDeps.toml")
  .requiredOption("-o, --output <dir>", "Output directory for generated files")
  .option("-r, --react", "Generate React Hooks instead of Vanilla JS strings")
  .action(async (options) => {
    const configPath = path.resolve(options.config);
    const outputDir = path.resolve(options.output);

    if (!fs.existsSync(configPath)) {
      console.error(`❌ Error: Config file not found at ${configPath}`);
      process.exit(1);
    }

    // 1. Read and Parse TOML
    const tomlString = await fs.readFile(configPath, "utf-8");
    const rawConfig = toml.parse(tomlString) as any;

    if (!rawConfig.contracts || Object.keys(rawConfig.contracts).length === 0) {
      console.error("❌ Error: No contracts defined in AxiomDeps.toml.");
      process.exit(1);
    }

    const multiIr: MultiIR = {};
    const projectRoot = path.dirname(configPath); // Frontend project root

    // 2. Loop through contracts
    for (const [namespace, contract] of Object.entries(rawConfig.contracts)) {
      // Rust CLI safely copies files to `public/[namespace].axiom`
      const axiomFilePath = path.resolve(
        projectRoot,
        `public/${namespace}.axiom`,
      );

      if (!fs.existsSync(axiomFilePath)) {
        console.warn(
          `⚠️ Warning: Contract file not found at ${axiomFilePath}. Skipping...`,
        );
        continue;
      }

      const rawFile = await fs.readJSON(axiomFilePath);
      if (!rawFile.ir) continue;

      multiIr[namespace] = normalizeIr(rawFile.ir);
      console.log(`✅ Loaded contract: [${namespace}] -> ${axiomFilePath}`);
    }

    await fs.ensureDir(outputDir);

    const modelsContent = generateModels(multiIr);
    await fs.writeFile(path.join(outputDir, "models.ts"), modelsContent);

    const sdkContent = generateSdk(multiIr, options.react);
    await fs.writeFile(path.join(outputDir, "sdk.ts"), sdkContent);

    console.log(
      `\n🎉 ATMX Multi-Contract SDK generated successfully in ${outputDir}`,
    );
  });

program.parse();
