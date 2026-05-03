#!/usr/bin/env node
import { Command } from "commander";
import * as fs from "fs-extra";
import * as path from "path";
import { AtmxMultiConfig, MultiIR } from "./types";
import { generateModels } from "./generators/model-generator";
import { generateSdk } from "./generators/sdk-generator";
import { normalizeIr } from "./generators/utils";

const program = new Command();

program
  .name("atmx")
  .description("Generate TypeScript SDK from an ATMX Multi-Contract Config")
  .version("0.2.0");

program
  .command("generate")
  .requiredOption("-c, --config <path>", "Path to the atmx.config.json file")
  .requiredOption("-o, --output <dir>", "Output directory for generated files")
  .option("-r, --react", "Generate React Hooks instead of Vanilla JS strings") // ✨ NEW
  .action(async (options) => {
    const configPath = path.resolve(options.config);
    const outputDir = path.resolve(options.output);

    if (!fs.existsSync(configPath)) {
      console.error(`❌ Error: Config file not found at ${configPath}`);
      process.exit(1);
    }

    // 1. Read the Config File
    const rawConfig: AtmxMultiConfig = await fs.readJSON(configPath);

    if (!rawConfig.contracts || Object.keys(rawConfig.contracts).length === 0) {
      console.error(
        "❌ Error: Invalid config file. Missing 'contracts' dictionary.",
      );
      process.exit(1);
    }

    const configDir = path.dirname(configPath);
    const multiIr: MultiIR = {};

    // 2. Loop through the contracts and parse the local .axiom files
    for (const [namespace, contract] of Object.entries(rawConfig.contracts)) {
      // Resolve the .axiom file path relative to where the config file is located
      // (e.g., if config is in /public, and file is "./auth.axiom", it looks in /public/auth.axiom)
      const axiomFilePath = path.resolve(configDir, contract.file);

      if (!fs.existsSync(axiomFilePath)) {
        console.warn(
          `⚠️ Warning: Contract file not found for namespace '${namespace}' at ${axiomFilePath}. Skipping...`,
        );
        continue;
      }

      const rawFile = await fs.readJSON(axiomFilePath);

      if (!rawFile.ir) {
        console.warn(
          `⚠️ Warning: Invalid .axiom file for namespace '${namespace}'. Missing 'ir' property. Skipping...`,
        );
        continue;
      }

      // Normalize the IR (snake_case -> camelCase) and store it by namespace
      multiIr[namespace] = normalizeIr(rawFile.ir);
      console.log(`✅ Loaded contract: [${namespace}] -> ${contract.file}`);
    }

    if (Object.keys(multiIr).length === 0) {
      console.error(
        "❌ Error: No valid contracts were loaded. Aborting generation.",
      );
      process.exit(1);
    }

    // Ensure output directory exists
    await fs.ensureDir(outputDir);

    // 3. Pass the MultiIR Map to the generators
    const modelsContent = generateModels(multiIr);
    await fs.writeFile(path.join(outputDir, "models.ts"), modelsContent);

    // ✨ NEW: Pass the react flag down
    const sdkContent = generateSdk(multiIr, options.react);
    await fs.writeFile(path.join(outputDir, "sdk.ts"), sdkContent);

    console.log(
      `\n🎉 ATMX Multi-Contract SDK generated successfully in ${outputDir}`,
    );
  });

program.parse();
