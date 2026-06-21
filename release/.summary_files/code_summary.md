Project Root: /Users/yashmakan/AxiomCore/AxiomCore/release
Project Structure:
```
.
|-- .release_todo
|-- atmx-cli
    |-- package-lock.json
    |-- package.json
    |-- src
        |-- generators
            |-- model-generator.ts
            |-- sdk-generator.ts
            |-- utils.ts
        |-- index.ts
        |-- templates
        |-- types.ts
    |-- tsconfig.json
|-- cli
    |-- .gitignore
    |-- justfile
    |-- scripts
        |-- build.sh
        |-- publish.sh
        |-- release.sh
|-- extractors
    |-- scripts
        |-- publish.sh
|-- homebrew-tap
    |-- Formula
        |-- axiom.rb
|-- justfile
|-- scripts
    |-- build_apple.sh
    |-- publish_apple.sh
    |-- publish_sdk.sh
    |-- wasm.sh

```

---
## File: atmx-cli/package.json

```json
{
  "name": "atmx-cli",
  "version": "0.107.0",
  "description": "",
  "main": "dist/index.js",
  "scripts": {
    "build": "tsc"
  },
  "bin": {
    "atmx": "./dist/index.js"
  },
  "keywords": [],
  "author": "",
  "license": "ISC",
  "type": "commonjs",
  "dependencies": {
    "@iarna/toml": "^2.2.5",
    "commander": "^14.0.3",
    "fs-extra": "^11.3.4"
  },
  "devDependencies": {
    "@types/fs-extra": "^11.0.4",
    "@types/iarna__toml": "^2.0.5",
    "@types/node": "^25.5.0",
    "ts-node": "^10.9.2",
    "typescript": "^5.9.3"
  }
}

```
---
## File: atmx-cli/src/generators/model-generator.ts

```ts
// FILE: atmx-cli/src/generators/model-generator.ts
import { AxiomEnum, AxiomModel, MultiIR } from "../types";
import { pascalCase, camelCase, mapTypeToTs } from "./utils";

export function generateModels(multiIr: MultiIR): string {
  const sections: string[] = [
    `// GENERATED CODE – DO NOT EDIT.\n/* eslint-disable @typescript-eslint/no-explicit-any */\n`,
    `/* eslint-disable @typescript-eslint/no-namespace */\n`, // ✨ FIX: Disable namespace lint error
  ];

  for (const [ns, ir] of Object.entries(multiIr)) {
    const camelNs = camelCase(ns);
    // ✨ FIX: Use proper TS namespaces
    sections.push(`export namespace ${camelNs} {`);

    const enumsList = Array.isArray(ir.enums)
      ? ir.enums
      : Object.values(ir.enums || {});
    const modelsList = Array.isArray(ir.models)
      ? ir.models
      : Object.values(ir.models || {});

    enumsList.forEach((en: any) => sections.push(generateEnum(en)));
    modelsList.forEach((model: any) =>
      sections.push(generateInterface(model, camelNs)),
    );

    sections.push(`}\n`);
  }

  sections.push(generateMappers(multiIr));
  return sections.join("\n");
}

function generateEnum(en: AxiomEnum): string {
  const name = pascalCase(en.name);
  const values = en.values.map((v) => `  ${pascalCase(v)}: "${v}"`).join(",\n");
  return `
  export const ${name} = {
  ${values}
  } as const;
  export type ${name} = typeof ${name}[keyof typeof ${name}];
  `;
}

function generateInterface(model: AxiomModel, ns: string): string {
  const name = pascalCase(model.name);
  const fields = model.fields
    .map((f) => {
      const type = mapTypeToTs(f.typeRef, ns);
      return `    ${camelCase(f.name)}${f.isOptional ? "?" : ""}: ${type};`;
    })
    .join("\n");

  return `
  export interface ${name} {
${fields}
  }
  `;
}

function generateMappers(multiIr: MultiIR): string {
  const lines: string[] = [`export const Mappers: Record<string, any> = {`];

  for (const [ns, ir] of Object.entries(multiIr)) {
    const camelNs = camelCase(ns);
    lines.push(`  ${camelNs}: {`);

    const modelsList = Array.isArray(ir.models)
      ? ir.models
      : Object.values(ir.models || {});

    modelsList.forEach((model: any) => {
      const name = pascalCase(model.name);
      const fullType = `${camelNs}.${name}`;

      lines.push(
        `    ${name}: {\n      fromJson: (json: any): ${fullType} => ({`,
      );
      model.fields.forEach((f: any) => {
        lines.push(
          `        ${camelCase(f.name)}: ${generateJsonLogic(f.typeRef, `json["${f.name}"]`, f.isOptional, "fromJson", camelNs)},`,
        );
      });
      lines.push(`      }),\n      toJson: (obj: any): any => ({`);
      model.fields.forEach((f: any) => {
        lines.push(
          `        "${f.name}": ${generateJsonLogic(f.typeRef, `obj.${camelCase(f.name)}`, f.isOptional, "toJson", camelNs)},`,
        );
      });
      lines.push(`      })\n    },`);
    });
    lines.push(`  },`);
  }
  lines.push(`};\n`);
  return lines.join("\n");
}

function generateJsonLogic(
  typeRef: any,
  access: string,
  isOpt: boolean,
  mode: "fromJson" | "toJson",
  ns: string,
): string {
  const wrap = (logic: string) =>
    isOpt ? `(${access} == null ? undefined : ${logic})` : logic;
  if (!typeRef || !typeRef.kind) return access;
  if (typeRef.kind === "dateTime")
    return mode === "fromJson"
      ? wrap(`new Date(${access})`)
      : wrap(`${access}.toISOString()`);
  if (typeRef.kind === "bytes")
    return mode === "fromJson"
      ? wrap(`new Uint8Array(${access})`)
      : wrap(`Array.from(${access})`);
  if (typeRef.kind === "named") {
    const name = pascalCase(typeRef.value);
    return wrap(
      `(Mappers.${ns}["${name}"] ? Mappers.${ns}["${name}"].${mode}(${access}) : ${access})`,
    );
  }
  if (typeRef.kind === "list")
    return wrap(
      `${access}.map((e: any) => ${generateJsonLogic(typeRef.value, "e", false, mode, ns)})`,
    );
  return access;
}

```
---
## File: atmx-cli/src/generators/sdk-generator.ts

```ts
import { AxiomIR, AxiomEndpoint, AxiomTypeRef } from "../types.js";

export interface ContractPayload {
  ir: AxiomIR;
  baseUrl: string;
  file: string;
}

// Helper to convert Axiom TypeRef to TypeScript Types
function getTsType(namespace: string, typeRef?: AxiomTypeRef): string {
  if (!typeRef) return "any";
  if (typeRef.kind === "named") {
    return `models.${namespace}.${typeRef.value}`;
  } else if (typeRef.kind === "list") {
    return `${getTsType(namespace, typeRef.value as AxiomTypeRef)}[]`;
  } else if (typeRef.kind === "primitive") {
    if (["int32", "int64", "float32", "float64"].includes(typeRef.value))
      return "number";
    if (typeRef.value === "bool") return "boolean";
    if (typeRef.value === "string") return "string";
  } else if (typeRef.kind === "void") {
    return "void";
  }
  return "any";
}

// Helper to get the correct deserializer function for a given TypeRef
function getDecoder(namespace: string, typeRef?: AxiomTypeRef): string {
  if (!typeRef) return `(json: any) => json`;

  if (typeRef.kind === "named") {
    return `models.Mappers.${namespace}.${typeRef.value}.fromJson`;
  } else if (
    typeRef.kind === "list" &&
    (typeRef.value as AxiomTypeRef).kind === "named"
  ) {
    const innerName = (typeRef.value as any).value;
    return `(json: any[]) => json.map(models.Mappers.${namespace}.${innerName}.fromJson)`;
  }

  return `(json: any) => json`;
}

export function generateSDKContent(
  contracts: Record<string, ContractPayload>,
  isReact: boolean,
): string {
  let content = `// GENERATED CODE – DO NOT EDIT.\n/* eslint-disable @typescript-eslint/no-explicit-any */\n/* eslint-disable @typescript-eslint/no-unused-vars */\n\n`;

  if (!isReact) {
    content += `import * as models from './models.js';\n\n`;
  } else {
    content += `import * as models from './models.js';\n`;
    content += `import { useAxiomQuery, useAxiomMutation, setAuthToken, clearAuthToken, axiomQueryManager } from "atmx-react";\n`;
    content += `import type { AxiomQueryDef } from "atmx-react";\n\n`;
  }

  // 1. Generate Individual Modules
  for (const [namespace, contract] of Object.entries(contracts)) {
    content += `export const ${namespace}Module = {\n`;
    content += `  axiom: {\n`;
    content += `    setAuthToken(methodName: string, token: string) {\n`;
    if (isReact) {
      content += `      setAuthToken("${namespace}", methodName, token);\n`;
    } else {
      content += `      (window as any).atmx?.setAuthToken("${namespace}", methodName, token);\n`;
    }
    content += `    },\n`;
    content += `    clearAuthToken(methodName: string) {\n`;
    if (isReact) {
      content += `      clearAuthToken("${namespace}", methodName);\n`;
    } else {
      content += `      (window as any).atmx?.clearAuthToken("${namespace}", methodName);\n`;
    }
    content += `    },\n`;
    content += `    connect(methodName: string, args?: Record<string, any>) {\n`;
    if (isReact) {
      content += `      const def = (${namespace}Module as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);\n`;
      content += `      axiomQueryManager.connect(def);\n`;
    } else {
      content += `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';\n`;
      content += `      (window as any).atmx?.connect(\`${namespace}.\${methodName}(\${argsStr})\`);\n`;
    }
    content += `    },\n`;
    content += `    disconnect(methodName: string, args?: Record<string, any>) {\n`;
    if (isReact) {
      content += `      const def = (${namespace}Module as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);\n`;
      content += `      axiomQueryManager.disconnect(def);\n`;
    } else {
      content += `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';\n`;
      content += `      (window as any).atmx?.disconnect(\`${namespace}.\${methodName}(\${argsStr})\`);\n`;
    }
    content += `    },\n`;
    content += `    send(methodName: string, payload: any, args?: Record<string, any>) {\n`;
    if (isReact) {
      content += `      const def = (${namespace}Module as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);\n`;
      content += `      axiomQueryManager.send(def, payload);\n`;
    } else {
      content += `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';\n`;
      content += `      (window as any).atmx?.send(\`${namespace}.\${methodName}(\${argsStr})\`, payload);\n`;
    }
    content += `    }\n`;
    content += `  },\n\n`;

    // Extract endpoints handling objects vs arrays
    const endpoints: AxiomEndpoint[] = Array.isArray(contract.ir.endpoints)
      ? contract.ir.endpoints
      : Object.values(contract.ir.endpoints || {});

    for (const endpoint of endpoints) {
      const fnName = endpoint.name.replace(/_([a-z])/g, (g) =>
        g[1].toUpperCase(),
      );
      const capFnName = fnName.charAt(0).toUpperCase() + fnName.slice(1);

      if (isReact) {
        const tsType = getTsType(namespace, endpoint.returnType);
        const decoder = getDecoder(namespace, endpoint.returnType);

        content += `  get${capFnName}Def(\n`;
        content += `    args?: Record<string, any>,\n`;
        content += `  ): AxiomQueryDef<${tsType}> {\n`;
        content += `    return {\n`;
        content += `      namespace: "${namespace}",\n`;
        content += `      name: "${endpoint.name}",\n`;
        content += `      endpointId: ${endpoint.id || 0},\n`;
        content += `      method: "${endpoint.method}",\n`;
        content += `      path: "${endpoint.path}",\n`;
        content += `      args: args || {},\n`;
        content += `      decoder: ${decoder},\n`;
        content += `      serializer: (p: any) => p,\n`;
        content += `      isStream: ${endpoint.isStream ? "true" : "false"},\n`;
        content += `    };\n`;
        content += `  },\n`;

        if (endpoint.method === "GET" || endpoint.method === "WS") {
          content += `  use${capFnName}(options?: { enabled?: boolean }) {\n`;
          content += `    return useAxiomQuery<${tsType}>(\n`;
          content += `      this.get${capFnName}Def(),\n`;
          content += `      options,\n`;
          content += `    );\n`;
          content += `  },\n`;
        } else {
          content += `  use${capFnName}(options?: any) {\n`;
          content += `    return useAxiomMutation<${tsType}>(\n`;
          content += `      this.get${capFnName}Def(),\n`;
          content += `      options,\n`;
          content += `    );\n`;
          content += `  },\n`;
        }
      } else {
        // ✨ UPGRADED: Vanilla Web static compiler methods mapped to dynamic objects
        content += `  ${fnName}: Object.assign(\n`;
        content += `    (args?: Record<string, any>): string => {\n`;
        content += `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';\n`;
        content += `      return \`${namespace}.${endpoint.name}(\${argsStr})\`;\n`;
        content += `    },\n`;
        content += `    {\n`;
        content += `      invalidate(args?: Record<string, any>) {\n`;
        content += `        (window as any).atmx?.invalidate("${namespace}.${endpoint.name}", args);\n`;
        content += `      },\n`;
        content += `      setData(data: any, args?: Record<string, any>) {\n`;
        content += `        (window as any).atmx?.setQueryData("${namespace}.${endpoint.name}", args || {}, data);\n`;
        content += `      },\n`;
        content += `      mutate(payload: any = {}, args?: Record<string, any>): Promise<any> {\n`;
        content += `        return (window as any).atmx?.mutate("${namespace}.${endpoint.name}", args, payload);\n`;
        content += `      }\n`;
        content += `    }\n`;
        content += `  ),\n`;
      }
    }
    content += `};\n\n`;
  }

  // 2. Generate the Smart Proxy SDK
  content += `const internalSdk: Record<string, any> = {\n`;
  for (const namespace of Object.keys(contracts)) {
    content += `  ${namespace}: ${namespace}Module,\n`;
  }
  content += `};\n\n`;

  content += `// ✨ The Magic Proxy: Safely intercepts Alpine.js evaluations during boot!\n`;
  content += `export const sdk = new Proxy(internalSdk, {\n`;
  content += `  get(target: any, prop: string, receiver: any) {\n`;
  content += `    if (prop in target) {\n`;
  content += `      return Reflect.get(target, prop, receiver);\n`;
  content += `    }\n`;
  content += `    // Create a dynamic namespace proxy\n`;
  content += `    return new Proxy({}, {\n`;
  content += `      get(subTarget: any, subProp: string) {\n`;
  content += `        // Return a callable function that returns the string definition\n`;
  content += `        const routeFn = (args?: Record<string, any>) => {\n`;
  content += `          const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';\n`;
  content += `          return \`\${String(prop)}.\${String(subProp)}(\${argsStr})\`;\n`;
  content += `        };\n`;
  content += `        // Attach typed helper methods directly to the function!\n`;
  content += `        routeFn.invalidate = (args?: Record<string, any>) => {\n`;
  content += `          (window as any).atmx?.invalidate(\`\${String(prop)}.\${String(subProp)}\`, args);\n`;
  content += `        };\n`;
  content += `        routeFn.setData = (data: any, args?: Record<string, any>) => {\n`;
  content += `          (window as any).atmx?.setQueryData(\`\${String(prop)}.\${String(subProp)}\`, args || {}, data);\n`;
  content += `        };\n`;
  content += `        routeFn.mutate = (payload: any = {}, args?: Record<string, any>): Promise<any> => {\n`;
  content += `          return (window as any).atmx?.mutate(\`\${String(prop)}.\${String(subProp)}\`, args, payload);\n`;
  content += `        };\n`;
  content += `        return routeFn;\n`;
  content += `      }\n`;
  content += `    });\n`;
  content += `  }\n`;
  content += `});\n\n`;

  content += `// Auto-attach to window for Alpine.js immediate hydration\n`;
  content += `if (typeof window !== "undefined") {\n`;
  content += `  (window as any).sdk = sdk;\n`;
  content += `}\n\n`;

  // 3. Generate Default Config
  content += `export const AxiomDefaultConfig = {\n`;
  content += `  contracts: {\n`;
  for (const [ns, def] of Object.entries(contracts)) {
    // Determine file path or default to namespace
    const contractPath = def.file ? def.file : `/${ns}.axiom`;

    content += `    "${ns}": {\n`;
    content += `      contractUrl: "${contractPath}",\n`;
    content += `      baseUrl: "${def.baseUrl}"\n`;
    content += `    },\n`;
  }
  content += `  }\n`;
  content += `};\n`;

  return content;
}

```
---
## File: atmx-cli/src/generators/utils.ts

```ts
// FILE: atmx-cli/src/generators/utils.ts
export function pascalCase(str: string): string {
  if (!str) return "";
  return str
    .split(/[_\-\s]+/)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("");
}

export function camelCase(str: string): string {
  const pascal = pascalCase(str);
  return pascal.charAt(0).toLowerCase() + pascal.slice(1);
}

export function normalizeIr(obj: any): any {
  if (Array.isArray(obj)) return obj.map(normalizeIr);
  if (obj !== null && typeof obj === "object") {
    const newObj: any = {};
    for (const key of Object.keys(obj)) {
      const camelKey = key.replace(/_([a-z])/g, (g) => g[1].toUpperCase());
      newObj[camelKey] = normalizeIr(obj[key]);
    }
    if (
      newObj.endpoints &&
      typeof newObj.endpoints === "object" &&
      !Array.isArray(newObj.endpoints)
    ) {
      newObj.endpoints = Object.values(newObj.endpoints);
    }
    if (
      newObj.models &&
      typeof newObj.models === "object" &&
      !Array.isArray(newObj.models)
    ) {
      newObj.models = Object.values(newObj.models);
    }
    if (
      newObj.enums &&
      typeof newObj.enums === "object" &&
      !Array.isArray(newObj.enums)
    ) {
      newObj.enums = Object.values(newObj.enums);
    }
    if (Array.isArray(newObj.models)) {
      newObj.models = newObj.models.map((model: any) => {
        if (
          model.fields &&
          typeof model.fields === "object" &&
          !Array.isArray(model.fields)
        ) {
          model.fields = Object.values(model.fields);
        }
        return model;
      });
    }
    return newObj;
  }
  return obj;
}

export function mapTypeToTs(typeRef: any, ns?: string): string {
  if (!typeRef || !typeRef.kind) return "any";

  switch (typeRef.kind) {
    case "string":
      return "string";
    case "int32":
    case "int64":
    case "float32":
    case "float64":
      return "number";
    case "bool":
      return "boolean";
    case "dateTime":
      return "Date";
    case "bytes":
      return "Uint8Array";
    case "void":
      return "void";
    case "json":
      return "any";
    case "named":
      const name = pascalCase(typeRef.value);
      return ns ? `${ns}.${name}` : name;
    case "list":
      return `${mapTypeToTs(typeRef.value, ns)}[]`;
    case "map":
      const valType = typeRef.value?.[1]
        ? mapTypeToTs(typeRef.value[1], ns)
        : "any";
      return `Record<string, ${valType}>`;
    default:
      return "any";
  }
}

```
---
## File: atmx-cli/src/index.ts

```ts
#!/usr/bin/env node
import { Command } from "commander";
import * as fs from "fs-extra";
import * as path from "path";
import * as toml from "@iarna/toml";
import { MultiIR } from "./types";
import { generateModels } from "./generators/model-generator";
// ✨ FIX: Import generateSDKContent and ContractPayload
import {
  generateSDKContent,
  ContractPayload,
} from "./generators/sdk-generator";
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
    const generatorPayload: Record<string, ContractPayload> = {}; // ✨ NEW: Payload for SDK
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

      // ✨ NEW: Combine IR with TOML config for the SDK generator
      generatorPayload[namespace] = {
        ir: multiIr[namespace],
        baseUrl: (contract as any).base_url || "http://localhost:8080",
        file: `/${namespace}.axiom`,
      };

      console.log(`✅ Loaded contract: [${namespace}] -> ${axiomFilePath}`);
    }

    await fs.ensureDir(outputDir);

    // 3. Generate Models (Needs raw MultiIR)
    const modelsContent = generateModels(multiIr);
    await fs.writeFile(path.join(outputDir, "models.ts"), modelsContent);

    // 4. Generate SDK (Needs enriched ContractPayload)
    const sdkContent = generateSDKContent(generatorPayload, options.react);
    await fs.writeFile(path.join(outputDir, "sdk.ts"), sdkContent);

    console.log(
      `\n🎉 ATMX Multi-Contract SDK generated successfully in ${outputDir}`,
    );
  });

program.parse();

```
---
## File: atmx-cli/src/types.ts

```ts
export interface AxiomIR {
  serviceName: string;
  endpoints: AxiomEndpoint[];
  models: Record<string, AxiomModel>;
  enums: Record<string, AxiomEnum>;
}

export interface AxiomEndpoint {
  id: number;
  name: string;
  path: string;
  method: string;
  parameters: AxiomParameter[];
  returnType: AxiomTypeRef;
  returnIsOptional: boolean;
  isStream: boolean;
}

export interface AxiomParameter {
  name: string;
  source: "path" | "query" | "body";
  typeRef: AxiomTypeRef;
  isOptional: boolean;
}

export type AxiomTypeRef =
  | { kind: "primitive" | "named"; value: string }
  | { kind: "list"; value: AxiomTypeRef }
  | { kind: "map"; value: [AxiomTypeRef, AxiomTypeRef] }
  | { kind: "void" };

export interface AxiomModel {
  name: string;
  fields: AxiomField[];
}

export interface AxiomField {
  name: string;
  typeRef: AxiomTypeRef;
  isOptional: boolean;
}

export interface AxiomEnum {
  name: string;
  values: string[];
}

export interface AtmxContractConfig {
  file: string; // Path relative to the config file (e.g., "./auth.axiom")
  baseUrl: string; // The URL for runtime (not used during code generation, but part of schema)
}

export interface AtmxMultiConfig {
  contracts: Record<string, AtmxContractConfig>;
}

// A Map holding the normalized IR for each contract
export type MultiIR = Record<string, AxiomIR>;

```
---
## File: cli/justfile

```
default:
    @just --list

build:
    ./scripts/build.sh

release:
    ./scripts/release.sh

publish version:
    ./scripts/publish.sh {{version}}

all version:
    ./scripts/build.sh {{version}}
    ./scripts/release.sh
    ./scripts/publish.sh {{version}}

```
---
## File: cli/scripts/build.sh

```sh
#!/bin/bash
set -e

CLI_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$CLI_ROOT/dist"
AXIOM_RUST_PATH="$CLI_ROOT/../../cli" # Points to AxiomCore/cli

mkdir -p "$DIST_DIR"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "🛠️  Starting Axiom CLI Build Process"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

VERSION_ARG=$1
if [ ! -z "$VERSION_ARG" ]; then
    CLEAN_VERSION="${VERSION_ARG#v}"
    CARGO_FILE="$AXIOM_RUST_PATH/Cargo.toml"

    if [ -f "$CARGO_FILE" ]; then
        echo "🔖 Updating Cargo.toml version to $CLEAN_VERSION"
        sed -i '' "s/^version = \".*\"/version = \"$CLEAN_VERSION\"/" "$CARGO_FILE"
    else
        echo "❌ Cargo.toml not found at $CARGO_FILE"
        exit 1
    fi
fi

echo "🦀 Building Axiom (Rust) in $AXIOM_RUST_PATH..."
cd "$AXIOM_RUST_PATH"
cargo build --release

# Copy the binary to our orchestration dist folder
cp "target/release/axiom-cli" "$DIST_DIR/axiom"

echo "✅ Axiom built successfully."
ls -lh "$DIST_DIR/axiom"

```
---
## File: cli/scripts/publish.sh

```sh

#!/bin/bash
set -e

VERSION=$1
if [ -z "$VERSION" ]; then
    echo "❌ Error: No version provided. Usage: ./publish.sh v0.1.0"
    exit 1
fi
PLAIN_VERSION="${VERSION#v}"

CLI_REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RELEASE_DIR="$CLI_REPO_DIR/release"
TAP_REPO_DIR="$(cd "$CLI_REPO_DIR/../homebrew-tap" && pwd)"

OS="macos"
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) ARCH="amd64" ;;
    arm64|aarch64) ARCH="arm64" ;;
esac
PLATFORM="${OS}-${ARCH}"

AXIOM_TAR="$RELEASE_DIR/axiom-${PLATFORM}.tar.gz"
if [[ ! -f "$AXIOM_TAR" ]]; then
    echo "❌ Error: Tarball not found at $AXIOM_TAR"
    exit 1
fi

AXIOM_SHA=$(shasum -a 256 "$AXIOM_TAR" | awk '{print $1}')

echo "🚀 Creating GitHub Release $VERSION..."
# Assumes you are running this from the main AxiomCore repo
git add .
git commit -m "Release $VERSION" || echo "Nothing to commit"
git push origin main

gh release create "$VERSION" "$AXIOM_TAR" --title "$VERSION" --notes "Automated Release" || echo "Release might already exist, uploading asset..."
gh release upload "$VERSION" "$AXIOM_TAR" --clobber

echo "🍺 Updating Homebrew Tap..."
FORMULA_FILE="$TAP_REPO_DIR/Formula/axiom.rb"

sed -i '' "s|releases/download/.*/axiom-macos-arm64.tar.gz|releases/download/${VERSION}/axiom-macos-arm64.tar.gz|g" "$FORMULA_FILE"
sed -i '' "s|sha256 \".*\"|sha256 \"${AXIOM_SHA}\"|g" "$FORMULA_FILE"
sed -i '' "s|version \".*\"|version \"${PLAIN_VERSION}\"|g" "$FORMULA_FILE"

cd "$TAP_REPO_DIR"
git add Formula/axiom.rb
git commit -m "Update Axiom to $VERSION" || true
git push origin main

echo "✅ Successfully published Axiom CLI $VERSION!"

```
---
## File: cli/scripts/release.sh

```sh
#!/bin/bash
set -e

CLI_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$CLI_ROOT/dist"
RELEASE_DIR="$CLI_ROOT/release"

mkdir -p "$RELEASE_DIR"

# Detect platform for naming the tarball
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

# FIX: Map 'darwin' kernel name to 'macos' for consistent naming
if [ "$OS" == "darwin" ]; then
    OS="macos"
fi

case "$ARCH" in
    x86_64)       ARCH="amd64" ;;
    arm64|aarch64) ARCH="arm64" ;;
    *)            ARCH="arm64" ;;
esac

PLATFORM="${OS}-${ARCH}"

echo "📦 Packaging Axiom CLI for $PLATFORM..."
cd "$DIST_DIR"

if [ ! -f "axiom" ]; then
    echo "❌ Error: axiom binary not found in $DIST_DIR."
    exit 1
fi

tar -czf "$RELEASE_DIR/axiom-${PLATFORM}.tar.gz" axiom
cd "$CLI_ROOT"

echo "✅ Checksum generated:"
shasum -a 256 "$RELEASE_DIR/axiom-${PLATFORM}.tar.gz"

```
---
## File: extractors/scripts/publish.sh

```sh
#!/bin/bash
set -e

EXTRACTOR_NAME=$1
VERSION=$2

if [ -z "$EXTRACTOR_NAME" ] || [ -z "$VERSION" ]; then
    echo "❌ Usage: ./publish.sh <extractor-name> <version-tag>"
    echo "Example: ./publish.sh axiom-fastapi v0.1.0"
    echo "Example: ./publish.sh axiom-go-extractor v0.1.0"
    exit 1
fi

# --- Resolve paths safely (independent of where script is run) ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

echo "📁 SCRIPT_DIR: $SCRIPT_DIR"
echo "📁 REPO_ROOT: $REPO_ROOT"

# --- Platform detection ---
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64) ARCH="amd64" ;;
    arm64|aarch64) ARCH="arm64" ;;
esac

PLATFORM="${OS}-${ARCH}"
ASSET_NAME="${EXTRACTOR_NAME}-${PLATFORM}"
BINARY_PATH=""

# ============================================================
# 🐹 GO EXTRACTOR
# ============================================================
if [ "$EXTRACTOR_NAME" == "axiom-go-extractor" ]; then
    GO_DIR="$REPO_ROOT/axiom-extractor/extractors/go"

    echo "🐹 Building Go Extractor '$EXTRACTOR_NAME' for $PLATFORM..."
    echo "📂 GO_DIR: $GO_DIR"

    if [[ ! -d "$GO_DIR" ]]; then
        echo "❌ Go extractor directory not found: $GO_DIR"
        exit 1
    fi

    cd "$GO_DIR"

    go mod tidy

    mkdir -p dist
    go build -o "dist/$EXTRACTOR_NAME" ./cmd/axiom-go-extractor

    BINARY_PATH="$GO_DIR/dist/$EXTRACTOR_NAME"

# ============================================================
# 🐍 PYTHON EXTRACTORS
# ============================================================
elif [[ "$EXTRACTOR_NAME" == axiom-* ]]; then
    SAFE_NAME="${EXTRACTOR_NAME//-/_}"

    EXTRACTOR_DIR="$REPO_ROOT/axiom-extractor/extractors/python/frameworks/$SAFE_NAME"

    echo "🐍 Building Python Extractor '$EXTRACTOR_NAME' for $PLATFORM..."
    echo "📂 EXTRACTOR_DIR: $EXTRACTOR_DIR"

    if [[ ! -d "$EXTRACTOR_DIR" ]]; then
        echo "❌ Extractor directory not found: $EXTRACTOR_DIR"
        exit 1
    fi

    cd "$EXTRACTOR_DIR"

    poetry install

    poetry run pyinstaller --onefile \
        --name "$EXTRACTOR_NAME" \
        --paths src/ \
        --collect-all fastapi \
        --collect-all pydantic \
        --collect-all pydantic_core \
        --collect-all starlette \
        build_entrypoint.py

    BINARY_PATH="$EXTRACTOR_DIR/dist/$EXTRACTOR_NAME"

else
    echo "❌ Unknown extractor type: $EXTRACTOR_NAME"
    exit 1
fi

# ============================================================
# ✅ VALIDATE BUILD
# ============================================================
echo "🔍 Checking binary at: $BINARY_PATH"

if [ ! -f "$BINARY_PATH" ]; then
    echo "❌ Build failed. Binary not found at $BINARY_PATH"
    exit 1
fi

# Move binary to repo root for upload
cd "$REPO_ROOT"
mv "$BINARY_PATH" "$ASSET_NAME"

TARGET_REPO="AxiomCore/AxiomCore"

echo "🚀 Uploading $ASSET_NAME to GitHub Release $VERSION on $TARGET_REPO..."

# Create release if it doesn't exist
gh release create "$VERSION" \
    --repo "$TARGET_REPO" \
    --title "$VERSION" \
    --notes "Unified Axiom Extractor Release" 2>/dev/null || true

# Upload asset
gh release upload "$VERSION" "$ASSET_NAME" \
    --repo "$TARGET_REPO" \
    --clobber

echo "✅ Published: $ASSET_NAME to $TARGET_REPO release $VERSION"

```
---
## File: homebrew-tap/Formula/axiom.rb

```rb
class Axiom < Formula
  desc "Axiom CLI - Unified Configuration and API SDK Generator"
  homepage "https://github.com/AxiomCore/AxiomCore"
  url "https://github.com/AxiomCore/AxiomCore/releases/download/v0.107.0/axiom-macos-arm64.tar.gz"
  sha256 "2de47b32325efc5e36c751d3da297c7fd0d6d0d25e99a87bdc1480271439705f"
  version "0.107.0"

  def install
    bin.install "axiom"
  end

  test do
    assert_match "Usage", shell_output("#{bin}/axiom --help")
  end
end

```
---
## File: justfile

```
default:
    @just --list

build-runtime:
    @echo "🛠 Building Runtimes (WASM + iOS/macOS)..."
    ./scripts/wasm.sh
    ./scripts/build_apple.sh

build-axiom version:
    ./cli/scripts/build.sh {{version}}

package-axiom:
    ./cli/scripts/release.sh

publish-axiom version:
    ./cli/scripts/publish.sh {{version}}

release-axiom version: (build-axiom version) package-axiom (publish-axiom version)

build-apple version:
    ./scripts/build_apple.sh

publish-apple version:
    ./scripts/publish_apple.sh {{version}}

release-apple version: (build-apple version) (publish-apple version)

publish-sdk sdk version:
    ./scripts/publish_sdk.sh {{sdk}} {{version}}

release-atmx version:
    #!/usr/bin/env bash
    set -e
    CLEAN_VERSION=$(echo "{{version}}" | sed 's/^v//')
    JUSTFILE_DIR="{{justfile_directory()}}"

    echo "📦 Preparing atmx-web core v$CLEAN_VERSION..."
    cd "$JUSTFILE_DIR/../../axiom-sdk/web/atmx"
    npm version $CLEAN_VERSION --no-git-tag-version

    echo "📝 Updating ATMX_VERSION constant in src/index.ts..."
    sed -i '' "s/export const ATMX_VERSION = \".*\";/export const ATMX_VERSION = \"$CLEAN_VERSION\";/" src/index.ts

    echo "🛠 Building atmx-web..."
    npm install
    npm run build

    echo "🚀 Publishing atmx-web to NPM..."
    npm publish --access public || echo "⚠️ NPM Publish failed (maybe already exists?)"

    echo "☁️ Uploading atmx-web to Cloudflare R2..."
    chmod +x scripts/upload.sh
    ./scripts/upload.sh

    # -----------------------------------------------------

    echo "📦 Building and Publishing atmx-react v$CLEAN_VERSION to NPM..."
    cd "$JUSTFILE_DIR/../../axiom-sdk/web/atmx-react"
    npm version $CLEAN_VERSION --no-git-tag-version

    echo "📝 Updating atmx-web dependency version in package.json..."
    sed -i '' "s/\"atmx-web\": \".*\"/\"atmx-web\": \"^$CLEAN_VERSION\"/" package.json

    npm install
    npm run build
    npm publish --access public || echo "⚠️ NPM Publish failed (maybe already exists?)"

    # -----------------------------------------------------

    echo "📦 Building and Publishing atmx-cli v$CLEAN_VERSION to NPM..."
    cd "$JUSTFILE_DIR/atmx-cli"
    npm version $CLEAN_VERSION --no-git-tag-version
    npm install
    npm run build
    npm publish --access public || echo "⚠️ NPM Publish failed (maybe already exists?)"

    echo "✅ ATMX Ecosystem published successfully!"

release-extractor name version:
    ./extractors/scripts/publish.sh {{name}} {{version}}

release-all version:
    @echo "🚀 Starting Full AxiomCore Release for v{{version}}..."

    # 1. Update SDK Versions in source code (pubspecs & Rust utils.rs) before compiling!
    ./scripts/publish_sdk.sh flutter {{version}} --skip-publish

    # 2. Build Runtimes (Wasm + Apple Framework)
    just build-runtime

    # 3. Build & Release Axiom CLI (Now has updated SDK versions compiled in)
    just release-axiom {{version}}

    # 4. Publish Apple Framework + Flutter SDKs to pub.dev
    @echo "📦 Publishing Apple SDK..."
    @if just publish-apple {{version}}; then \
        echo "✅ Apple SDK published successfully"; \
    else \
        echo "⚠️ Apple publish failed (likely rate limit). Skipping for now."; \
        echo "publish-apple {{version}}" >> .release_todo; \
    fi

    # 5. Release Web SDK
    just release-atmx {{version}}

    # 6. Release Extractors
    @echo "📦 Releasing Extractors..."
    just release-extractor axiom-fastapi {{version}}
    just release-extractor axiom-go-extractor {{version}}

    @echo "================================================="
    @echo "🎉 All AxiomCore components released successfully for v{{version}}!"
    @echo "⚠️ NOTE: Don't forget to commit the version bumps in axiom-sdk!"
    @echo "   cd ../../axiom-sdk && git add . && git commit -m 'chore: bump sdk to {{version}}' && git push"

```
---
## File: scripts/build_apple.sh

```sh
#!/bin/bash
set -e

export MACOSX_DEPLOYMENT_TARGET=11.0
export IPHONEOS_DEPLOYMENT_TARGET=13.0

# --- Absolute Path Resolution ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

RUNTIME_DIR="$REPO_ROOT/../axiom-runtime"
LIB_NAME="libaxiom_runtime.a"
FRAMEWORK_NAME="AxiomRuntime"

INCLUDE_DIR="$RUNTIME_DIR/include"
DIST_DIR="$RUNTIME_DIR/dist"
TARGET_DIR="$RUNTIME_DIR/target"

echo "🚀 Starting Universal Apple Build Process..."
cd "$RUNTIME_DIR"

# 1. Generate Static Headers
echo "📝 Writing static C headers..."
mkdir -p "$INCLUDE_DIR"
cat <<EOF > "$INCLUDE_DIR/axiom.h"
#ifndef AXIOM_RUNTIME_H
#define AXIOM_RUNTIME_H
#include <stdint.h>
#include <stdbool.h>

typedef struct { const uint8_t* ptr; uint64_t len; } AxiomString;
typedef struct { uint8_t* ptr; uint64_t len; } AxiomBuffer;

typedef enum {
    Success = 0,
    UnknownError = 1,
    RequestParsingFailed = 2,
    NetworkError = 3,
    ResponseDeserializationFailed = 4,
    UnknownEndpoint = 5,
    InvalidContract = 10,
    RuntimeTooOld = 11,
    ContractNotLoaded = 12
} FfiError;

typedef struct {
    uint64_t request_id;
    int32_t error_code;
    AxiomBuffer data;
    AxiomBuffer error_message; // Added for updated FFI signature
} AxiomResponseBuffer;

typedef void (*AxiomCallback)(AxiomResponseBuffer* response);
typedef void (*AxiomAuthCallback)(uint64_t request_id);

void axiom_initialize(AxiomString base_url);
int32_t axiom_load_contract(AxiomString namespace, AxiomString base_url, AxiomBuffer contract_buf, AxiomString signature, AxiomString public_key);
void axiom_register_callback(AxiomCallback callback);
void axiom_register_auth_provider(AxiomAuthCallback callback);
void axiom_provide_auth_token(uint64_t request_id, AxiomString token);
void axiom_free_buffer(AxiomBuffer buf);
void axiom_process_responses();
void axiom_call(uint64_t request_id, AxiomString namespace, uint32_t endpoint_id, AxiomString method, AxiomString path, AxiomString traceparent, AxiomString headers_json, AxiomBuffer input_buf);
void axiom_set_auth_token(AxiomString namespace, AxiomString method_name, AxiomString token);
void axiom_clear_auth_token(AxiomString namespace, AxiomString method_name);
void axiom_send_stream_message(uint64_t request_id, AxiomBuffer payload_buf);

#endif
EOF

cat <<EOF > "$INCLUDE_DIR/module.modulemap"
module AxiomRuntime {
    header "axiom.h"
    export *
}
EOF

# 2. Build Rust Targets
echo "🛠 Building Rust targets (iOS + macOS)..."
rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim \
                  aarch64-apple-darwin x86_64-apple-darwin

cargo build --release --target aarch64-apple-ios
cargo build --release --target x86_64-apple-ios
cargo build --release --target aarch64-apple-ios-sim

cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# 3. Create Universal Binaries (Lipo)
echo "🔗 Creating universal binaries..."

mkdir -p "$TARGET_DIR/ios-sim-universal"
lipo -create \
    "$TARGET_DIR/x86_64-apple-ios/release/$LIB_NAME" \
    "$TARGET_DIR/aarch64-apple-ios-sim/release/$LIB_NAME" \
    -output "$TARGET_DIR/ios-sim-universal/$LIB_NAME"

mkdir -p "$TARGET_DIR/macos-universal"
lipo -create \
    "$TARGET_DIR/x86_64-apple-darwin/release/$LIB_NAME" \
    "$TARGET_DIR/aarch64-apple-darwin/release/$LIB_NAME" \
    -output "$TARGET_DIR/macos-universal/$LIB_NAME"

# 4. Create XCFramework
echo "📦 Packaging Universal XCFramework..."
rm -rf "$DIST_DIR/$FRAMEWORK_NAME.xcframework"
mkdir -p "$DIST_DIR"

xcodebuild -create-xcframework \
    -library "$TARGET_DIR/aarch64-apple-ios/release/$LIB_NAME" \
    -headers "$INCLUDE_DIR" \
    -library "$TARGET_DIR/ios-sim-universal/$LIB_NAME" \
    -headers "$INCLUDE_DIR" \
    -library "$TARGET_DIR/macos-universal/$LIB_NAME" \
    -headers "$INCLUDE_DIR" \
    -output "$DIST_DIR/$FRAMEWORK_NAME.xcframework"

# 5. COMPRESS FOR DISTRIBUTION
echo "🗜 Zipping XCFramework for remote distribution..."
cd "$DIST_DIR"
# -y preserves symlinks which Apple Frameworks require
zip -ryq "$FRAMEWORK_NAME.xcframework.zip" "$FRAMEWORK_NAME.xcframework"
cd - > /dev/null

echo "-------------------------------------------"
echo "✅ Universal Framework Zipped at $DIST_DIR/$FRAMEWORK_NAME.xcframework.zip"

```
---
## File: scripts/publish_apple.sh

```sh
#!/bin/bash
set -e

VERSION=$1
if [ -z "$VERSION" ]; then
    echo "❌ Error: No version provided. Usage: ./publish_apple.sh v0.1.0"
    exit 1
fi
# Strip the "v" prefix for places that just need the raw number
CLEAN_VERSION="${VERSION#v}"
TAGGED_VERSION="v${CLEAN_VERSION}"

RELEASE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ZIP_PATH="$RELEASE_ROOT/dist/AxiomRuntime.xcframework.zip"

if [ ! -f "$ZIP_PATH" ]; then
    echo "❌ Error: Zip file not found at $ZIP_PATH. Run 'just build-apple' first."
    exit 1
fi

echo "🧮 Calculating SHA256 checksum..."
ZIP_SHA=$(shasum -a 256 "$ZIP_PATH" | awk '{print $1}')
echo "   SHA256: $ZIP_SHA"

# ==========================================
# 1. UPDATE FLUTTER PODSPECS
# ==========================================
echo "📝 Updating Flutter Podspecs to version $CLEAN_VERSION..."
PODSPEC_IOS="$RELEASE_ROOT/../../axiom-sdk/flutter/axiom_flutter/ios/axiom_flutter.podspec"
PODSPEC_MACOS="$RELEASE_ROOT/../../axiom-sdk/flutter/axiom_flutter/macos/axiom_flutter.podspec"

# macOS sed syntax to replace `s.version = '...'`
sed -i '' "s/s\.version[[:space:]]*=[[:space:]]*'.*'/s.version          = '${CLEAN_VERSION}'/g" "$PODSPEC_IOS"
sed -i '' "s/s\.version[[:space:]]*=[[:space:]]*'.*'/s.version          = '${CLEAN_VERSION}'/g" "$PODSPEC_MACOS"

# ==========================================
# 2. UPDATE SWIFT PACKAGE
# ==========================================
echo "📝 Updating Swift Package.swift to version $CLEAN_VERSION..."
PACKAGE_SWIFT="$RELEASE_ROOT/../../axiom-sdk/swift/Package.swift"

# Replace the URL to point to the new version tag
sed -i '' "s|url: \"https://github.com/AxiomCore/AxiomCore/releases/download/.*/AxiomRuntime.xcframework.zip\"|url: \"https://github.com/AxiomCore/AxiomCore/releases/download/v${CLEAN_VERSION}/AxiomRuntime.xcframework.zip\"|g" "$PACKAGE_SWIFT"

# Replace the checksum with the actual calculated SHA256
sed -i '' "s/checksum: \".*\"/checksum: \"${ZIP_SHA}\"/g" "$PACKAGE_SWIFT"

echo "🚀 Creating GitHub Release $TAGGED_VERSION if it doesn't exist..."
gh release create "$TAGGED_VERSION" \
  --repo AxiomCore/AxiomCore \
  --title "$TAGGED_VERSION" \
  --notes "Release $TAGGED_VERSION" \
  2>/dev/null || echo "ℹ️  Release $TAGGED_VERSION already exists, skipping create..."

echo "🚀 Uploading $ZIP_PATH to GitHub Release $TAGGED_VERSION..."
gh release upload "$TAGGED_VERSION" "$ZIP_PATH" --repo AxiomCore/AxiomCore --clobber

echo "✅ Apple framework published and SDK definitions updated!"


# ==========================================
# 4. PUBLISH SDK
# ==========================================
echo "🚀 Triggering Flutter SDK Publish..."
"$RELEASE_ROOT/scripts/publish_sdk.sh" flutter "$VERSION"

echo "✅ Apple framework published and SDK definitions updated!"

```
---
## File: scripts/publish_sdk.sh

```sh
#!/bin/bash
set -e

SDK=$1
VERSION=$2
if [ -z "$VERSION" ]; then
    echo "❌ Usage: ./publish_sdk.sh <sdk> <v0.1.0>"
    exit 1
fi
CLEAN_VERSION="${VERSION#v}"

RELEASE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$RELEASE_ROOT/../.."

if [ "$SDK" == "flutter" ]; then
    echo "📦 Preparing Flutter SDK v$CLEAN_VERSION..."

    # 1. Update Rust CLI Template Constants so the next `cargo build` uses the new version
    UTILS_RS="$REPO_ROOT/axiom-build/src/core/utils.rs"
    if [ -f "$UTILS_RS" ]; then
        echo "📝 Updating Flutter SDK version in axiom-build/src/core/utils.rs..."
        sed -i '' "s/const AXIOM_FLUTTER_VERSION: &str = \".*\";/const AXIOM_FLUTTER_VERSION: \&str = \"^${CLEAN_VERSION}\";/" "$UTILS_RS"
        sed -i '' "s/const AXIOM_FLUTTER_GENERATOR_VERSION: &str = \".*\";/const AXIOM_FLUTTER_GENERATOR_VERSION: \&str = \"^${CLEAN_VERSION}\";/" "$UTILS_RS"
    fi

    # 2. Publish Generator
    echo "📝 Updating axiom_flutter_generator pubspec.yaml..."
    cd "$REPO_ROOT/axiom-sdk/flutter/axiom_flutter_generator"
    sed -i '' "s/^version: .*/version: ${CLEAN_VERSION}/" pubspec.yaml

    # Skip actual publish if --skip-publish flag is passed (used for prepping files before build)
    if [ "$3" != "--skip-publish" ]; then
        echo "🚀 Publishing axiom_flutter_generator to pub.dev..."
        fvm dart pub publish --force
    fi

    # 3. Publish Main SDK
    echo "📝 Updating axiom_flutter pubspec.yaml..."
    cd "$REPO_ROOT/axiom-sdk/flutter/axiom_flutter"
    sed -i '' "s/^version: .*/version: ${CLEAN_VERSION}/" pubspec.yaml

    if [ "$3" != "--skip-publish" ]; then
        echo "🚀 Publishing axiom_flutter to pub.dev..."
        fvm dart pub publish --force
    fi

    echo "✅ Flutter SDK updated/published successfully!"
else
    echo "❌ Unknown SDK: $SDK"
    exit 1
fi

```
---
## File: scripts/wasm.sh

```sh
#!/bin/bash
set -e

# Make sure wasm-pack is installed
if ! command -v wasm-pack &> /dev/null
then
    echo "📦 wasm-pack not found. Installing..."
    cargo install wasm-pack
fi

# --- Absolute Path Resolution ---
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

RUNTIME_DIR="$REPO_ROOT/../axiom-runtime"
DIST_DIR="$RUNTIME_DIR/dist/wasm"

# Flutter Plugin Paths
FLUTTER_PLUGIN_ROOT="$REPO_ROOT/../axiom-sdk/flutter/axiom_flutter"
FLUTTER_WEB_ASSETS="$FLUTTER_PLUGIN_ROOT/lib/assets/wasm"

# ATMX JS Library Paths
ATMX_ROOT="$REPO_ROOT/../axiom-sdk/web/atmx"
ATMX_VENDOR_DIR="$ATMX_ROOT/src/core/vendor"

echo "🚀 Starting WebAssembly Build Process..."
cd "$RUNTIME_DIR"

# 1. Build using wasm-pack targeting no-modules
echo "🛠 Compiling Rust to WebAssembly (no-modules)..."
wasm-pack build --target no-modules --out-dir "$DIST_DIR" --release

# 2. Optimize Wasm (Optional)
if command -v wasm-opt &> /dev/null
then
    echo "⚡ Optimizing Wasm bundle..."
    wasm-opt -Oz \
      --enable-bulk-memory \
      --enable-nontrapping-float-to-int \
      --enable-sign-ext \
      --enable-mutable-globals \
      --strip-debug \
      --strip-producers \
      --dce \
      --vacuum \
      "$DIST_DIR/axiom_runtime_bg.wasm" -o "$DIST_DIR/axiom_runtime_bg.wasm"
else
    echo "⚠️ wasm-opt not found. Skipping optimization. (Install binaryen for smaller output)"
fi

# 3. Auto-Copy to Flutter Plugin assets folder
echo "🚚 Syncing Wasm to Flutter plugin..."
mkdir -p "$FLUTTER_WEB_ASSETS"
rm -rf "$FLUTTER_WEB_ASSETS"/*
cp "$DIST_DIR/axiom_runtime_bg.wasm" "$FLUTTER_WEB_ASSETS/axiom_runtime_bg.wasm"
cp "$DIST_DIR/axiom_runtime.js" "$FLUTTER_WEB_ASSETS/axiom_runtime.js"

# 4. Auto-Copy to ATMX
echo "🚚 Syncing Wasm to ATMX library..."
mkdir -p "$ATMX_VENDOR_DIR"
mkdir -p "$ATMX_ROOT/public"

# Keep JS glue code in vendor so Vite bundles it
rm -rf "$ATMX_VENDOR_DIR"/*
cp "$DIST_DIR/axiom_runtime.js" "$ATMX_VENDOR_DIR/axiom_runtime.js"

# Move WASM to public so Vite outputs it separately
cp "$DIST_DIR/axiom_runtime_bg.wasm" "$ATMX_ROOT/public/axiom_runtime.wasm"

echo "-------------------------------------------"
echo "✅ WebAssembly Sync Complete!"

```
---
