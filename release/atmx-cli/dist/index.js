#!/usr/bin/env node
"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
const commander_1 = require("commander");
const fs = __importStar(require("fs-extra"));
const path = __importStar(require("path"));
const toml = __importStar(require("@iarna/toml"));
const model_generator_1 = require("./generators/model-generator");
const sdk_generator_1 = require("./generators/sdk-generator");
const utils_1 = require("./generators/utils");
const program = new commander_1.Command();
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
    const rawConfig = toml.parse(tomlString);
    if (!rawConfig.contracts || Object.keys(rawConfig.contracts).length === 0) {
        console.error("❌ Error: No contracts defined in AxiomDeps.toml.");
        process.exit(1);
    }
    const multiIr = {};
    const projectRoot = path.dirname(configPath); // Frontend project root
    // 2. Loop through contracts
    for (const [namespace, contract] of Object.entries(rawConfig.contracts)) {
        // Rust CLI safely copies files to `public/[namespace].axiom`
        const axiomFilePath = path.resolve(projectRoot, `public/${namespace}.axiom`);
        if (!fs.existsSync(axiomFilePath)) {
            console.warn(`⚠️ Warning: Contract file not found at ${axiomFilePath}. Skipping...`);
            continue;
        }
        const rawFile = await fs.readJSON(axiomFilePath);
        if (!rawFile.ir)
            continue;
        multiIr[namespace] = (0, utils_1.normalizeIr)(rawFile.ir);
        console.log(`✅ Loaded contract: [${namespace}] -> ${axiomFilePath}`);
    }
    await fs.ensureDir(outputDir);
    const modelsContent = (0, model_generator_1.generateModels)(multiIr);
    await fs.writeFile(path.join(outputDir, "models.ts"), modelsContent);
    const sdkContent = (0, sdk_generator_1.generateSdk)(multiIr, options.react);
    await fs.writeFile(path.join(outputDir, "sdk.ts"), sdkContent);
    console.log(`\n🎉 ATMX Multi-Contract SDK generated successfully in ${outputDir}`);
});
program.parse();
