Project Root: /Users/yashmakan/AxiomCore/AxiomCore/release/atmx-cli
Project Structure:
```
.
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

```

---
## File: package.json

```json
{
  "name": "atmx-cli",
  "version": "0.62.0",
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
## File: src/generators/model-generator.ts

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
## File: src/generators/sdk-generator.ts

```ts
// FILE: atmx-cli/src/generators/sdk-generator.ts
import { AxiomEndpoint, AxiomParameter, MultiIR } from "../types";
import { pascalCase, camelCase, mapTypeToTs } from "./utils";

export function generateSdk(
  multiIr: MultiIR,
  isReact: boolean = false,
): string {
  const lines: string[] = [
    `// GENERATED CODE – DO NOT EDIT.`,
    `/* eslint-disable @typescript-eslint/no-explicit-any */`,
    `import * as models from './models';\n`,
  ];

  if (isReact) {
    // ✨ FIX: Auto-import the auth helpers to bind them to the module
    lines.push(
      `import { useAxiomQuery, useAxiomMutation, setAuthToken, clearAuthToken } from 'atmx-react';`,
    );
    lines.push(`import type { AxiomQueryDef } from 'atmx-react';\n`);
  }

  for (const [ns, ir] of Object.entries(multiIr)) {
    const camelNs = camelCase(ns);
    lines.push(`export const ${camelNs}Module = {`);

    // ✨ FIX: Safely bind auth tokens using the EXACT namespace string from the TOML file!
    if (isReact) {
      lines.push(`  setAuthToken(methodName: string, token: string) {`);
      lines.push(`    setAuthToken("${ns}", methodName, token);`);
      lines.push(`  },`);
      lines.push(`  clearAuthToken(methodName: string) {`);
      lines.push(`    clearAuthToken("${ns}", methodName);`);
      lines.push(`  },`);
    } else {
      lines.push(`  setAuthToken(methodName: string, token: string) {`);
      lines.push(
        `    (window as any).atmx?.setAuthToken("${ns}", methodName, token);`,
      );
      lines.push(`  },`);
      lines.push(`  clearAuthToken(methodName: string) {`);
      lines.push(
        `    (window as any).atmx?.clearAuthToken("${ns}", methodName);`,
      );
      lines.push(`  },`);
    }

    const endpointsMap = ir.endpoints || {};
    const endpoints = Array.isArray(endpointsMap)
      ? endpointsMap
      : Object.values(endpointsMap);

    endpoints.forEach((ep: any) => {
      lines.push(generateEndpointMethod(ep, ns, camelNs, isReact));
    });
    lines.push(`};\n`);
  }

  lines.push(`export const sdk = {`);
  for (const ns of Object.keys(multiIr)) {
    lines.push(`  ${camelCase(ns)}: ${camelCase(ns)}Module,`);
  }
  lines.push(`};\n`);

  lines.push(`export const AxiomDefaultConfig = {`);
  lines.push(`  contracts: {`);
  for (const ns of Object.keys(multiIr)) {
    lines.push(`    "${ns}": {`);
    lines.push(`      contractUrl: "/${ns}.axiom",`);
    lines.push(`      baseUrl: "http://localhost:8000"`);
    lines.push(`    },`);
  }
  lines.push(`  }`);
  lines.push(`};\n`);

  return lines.join("\n");
}

// FILE: atmx-cli/src/generators/sdk-generator.ts (Partial replacement)

function generateEndpointMethod(
  ep: AxiomEndpoint,
  ns: string,
  camelNs: string,
  isReact: boolean,
): string {
  const rawParams = ep.parameters || [];
  const params = Array.isArray(rawParams)
    ? rawParams
    : Object.values(rawParams);

  const isQuery = ep.method ? ep.method.toUpperCase() === "GET" : true;
  const rawReturnType = mapTypeToTs(ep.returnType, camelNs);
  const returnType =
    rawReturnType === "void" || rawReturnType === "any"
      ? rawReturnType
      : prefixModels(rawReturnType);

  // Map camelCase TS args back to original IR snake_case names!
  const argsMapping = params
    .map(
      (p: any) =>
        `if (args && '${camelCase(p.name)}' in args) { mappedArgs["${p.name}"] = (args as any)["${camelCase(p.name)}"]; delete mappedArgs["${camelCase(p.name)}"]; }`,
    )
    .join("\n    ");

  if (params.length === 0) {
    if (isReact) {
      const decLogic = generateLambda(ep.returnType, "fromJson", camelNs);
      return `
  get${pascalCase(ep.name)}Def(args?: Record<string, any>): AxiomQueryDef<${returnType}> {
    return {
      namespace: "${ns}", name: "${ep.name}", endpointId: ${ep.id},
      method: "${ep.method ? ep.method.toUpperCase() : "GET"}", path: "${ep.path}",
      args: args || {}, decoder: ${decLogic}, serializer: (p: any) => p, isStream: ${ep.isStream === true}
    };
  },
  use${pascalCase(ep.name)}${!isQuery ? "Mutation" : ""}(options?: { enabled?: boolean }) {
    ${isQuery ? `return useAxiomQuery<${returnType}>(this.get${pascalCase(ep.name)}Def(), options);` : `return useAxiomMutation<${returnType}, void | Record<string,any>>((a) => this.get${pascalCase(ep.name)}Def(a));`}
  },`;
    } else {
      return `
  ${camelCase(ep.name)}(args?: Record<string, any>): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return \`${ns}.${ep.name}(\${argsStr})\`;
  },`;
    }
  }

  const argType = `{ ${params.map((p: any) => `${camelCase(p.name)}${p.isOptional ? "?" : ""}: ${prefixModels(mapTypeToTs(p.typeRef, camelNs))}`).join(", ")} }`;

  if (isReact) {
    const bodyParam = params.find(
      (p: any) => p.source === "body",
    ) as AxiomParameter;
    const payloadLogic = bodyParam
      ? `const payload = (args as any)?.${camelCase(bodyParam.name)};`
      : `const payload = undefined;`;
    const decLogic = generateLambda(ep.returnType, "fromJson", camelNs);
    const serLogic = bodyParam
      ? generateLambda(bodyParam.typeRef, "toJson", camelNs)
      : `(p: any) => p`;

    return `
  get${pascalCase(ep.name)}Def(args?: ${argType}): AxiomQueryDef<${returnType}> {
    ${payloadLogic}
    const mappedArgs: any = { ...(args || {}) };
    ${argsMapping}

    return {
      namespace: "${ns}", name: "${ep.name}", endpointId: ${ep.id},
      method: "${ep.method ? ep.method.toUpperCase() : "GET"}", path: "${ep.path}",
      payload: payload, args: mappedArgs, decoder: ${decLogic}, serializer: ${serLogic}, isStream: ${ep.isStream === true}
    };
  },
  use${pascalCase(ep.name)}${!isQuery ? "Mutation" : ""}(args?: ${argType}, options?: { enabled?: boolean }) {
    ${isQuery ? `return useAxiomQuery<${returnType}>(this.get${pascalCase(ep.name)}Def(args), options);` : `return useAxiomMutation<${returnType}, ${argType}>((a) => this.get${pascalCase(ep.name)}Def(a || args));`}
  },`;
  } else {
    return `
  ${camelCase(ep.name)}(args?: ${argType}): string {
    const mappedArgs: any = { ...(args || {}) };
    ${argsMapping}
    const argsStr = Object.keys(mappedArgs).length > 0 ? JSON.stringify(mappedArgs) : '';
    return \`${ns}.${ep.name}(\${argsStr})\`;
  },`;
  }
}

function generateLambda(
  typeRef: any,
  mode: "fromJson" | "toJson",
  ns: string,
): string {
  if (!typeRef || !typeRef.kind || typeRef.kind === "void")
    return mode === "fromJson" ? `() => undefined` : `(p: any) => p`;
  if (typeRef.kind === "list" && typeRef.value?.kind === "named")
    return `(data: any[]) => data.map(models.Mappers.${ns}.${pascalCase(typeRef.value.value)}.${mode})`;
  if (typeRef.kind === "named")
    return `models.Mappers.${ns}.${pascalCase(typeRef.value)}.${mode}`;
  return `(data: any) => data`;
}

function prefixModels(type: string): string {
  const primitives = [
    "string",
    "number",
    "boolean",
    "Date",
    "Uint8Array",
    "void",
    "any",
  ];
  if (!type || primitives.includes(type)) return type;
  if (type.endsWith("[]")) return `${prefixModels(type.slice(0, -2))}[]`;
  if (type.startsWith("models.")) return type;
  return `models.${type}`;
}

```
---
## File: src/generators/utils.ts

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
## File: src/index.ts

```ts
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

```
---
## File: src/types.ts

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
## File: tsconfig.json

```json
{
    "compilerOptions": {
        "target": "ES2020",
        "module": "CommonJS",
        "lib": [
            "ES2020"
        ],
        "rootDir": "src",
        "outDir": "dist",
        "moduleResolution": "node",
        "esModuleInterop": true,
        "forceConsistentCasingInFileNames": true,
        "strict": true,
        "skipLibCheck": true
    },
    "include": [
        "src/**/*"
    ]
}
```
---
