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
    lines.push(`import { useAxiomQuery, useAxiomMutation } from 'atmx-react';`);
    lines.push(`import type { AxiomQueryDef } from 'atmx-react';\n`);
  }

  for (const [ns, ir] of Object.entries(multiIr)) {
    const camelNs = camelCase(ns);
    // ✨ FIX: Use an object literal instead of a Class to support React Hooks!
    lines.push(`export const ${camelNs}Module = {`);

    const endpointsMap = ir.endpoints || {};
    const endpoints = Array.isArray(endpointsMap)
      ? endpointsMap
      : Object.values(endpointsMap);

    endpoints.forEach((ep: any) => {
      lines.push(generateEndpointMethod(ep, ns, camelNs, isReact));
    });
    lines.push(`};\n`);
  }

  // Generate main SDK object
  lines.push(`export const sdk = {`);
  for (const ns of Object.keys(multiIr)) {
    lines.push(`  ${camelCase(ns)}: ${camelCase(ns)}Module,`);
  }
  lines.push(`};\n`);

  // Generate Config
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

  const argType =
    params.length > 0
      ? `{ ${params.map((p: any) => `${camelCase(p.name)}${p.isOptional ? "?" : ""}: ${prefixModels(mapTypeToTs(p.typeRef, camelNs))}`).join(", ")} }`
      : "void";

  const isQuery = ep.method ? ep.method.toUpperCase() === "GET" : true;
  const rawReturnType = mapTypeToTs(ep.returnType, camelNs);
  const returnType =
    rawReturnType === "void" || rawReturnType === "any"
      ? rawReturnType
      : prefixModels(rawReturnType);

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
  get${pascalCase(ep.name)}Def(args${params.length > 0 ? "?" : ""}: ${argType === "void" ? "any" : argType}): AxiomQueryDef<${returnType}> {
    ${payloadLogic}
    return {
      namespace: "${ns}",
      name: "${ep.name}",
      endpointId: ${ep.id},
      method: "${ep.method ? ep.method.toUpperCase() : "GET"}",
      path: "${ep.path}",
      payload: payload,
      args: args || {},
      decoder: ${decLogic},
      serializer: ${serLogic},
      isStream: ${ep.isStream === true}
    };
  },

  use${pascalCase(ep.name)}${!isQuery ? "Mutation" : ""}(${isQuery ? `args${params.length > 0 ? "?" : ""}: ${argType === "void" ? "any" : argType}, options?: { enabled?: boolean }` : ""}) {
    ${
      isQuery
        ? `return useAxiomQuery<${returnType}>(this.get${pascalCase(ep.name)}Def(args), options);`
        : `return useAxiomMutation<${returnType}, ${argType === "void" ? "void | Record<string,any>" : argType}>((args) => this.get${pascalCase(ep.name)}Def(args));`
    }
  },`;
  } else {
    return `
  ${camelCase(ep.name)}(args${params.length > 0 ? "?" : ""}: ${argType === "void" ? "any" : argType}): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
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
