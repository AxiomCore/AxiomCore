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

    // 👉 THIS LINE WAS MISSING IN THE PREVIOUS STEP!
    lines.push(`export const ${camelNs}Module = {`);

    lines.push(`  axiom: {`);
    if (isReact) {
      lines.push(`    setAuthToken(methodName: string, token: string) {`);
      lines.push(`      setAuthToken("${ns}", methodName, token);`);
      lines.push(`    },`);
      lines.push(`    clearAuthToken(methodName: string) {`);
      lines.push(`      clearAuthToken("${ns}", methodName);`);
      lines.push(`    },`);
      // ✨ FIX: Properly generate React WebSocket imperative methods!
      lines.push(
        `    connect(methodName: string, args?: Record<string, any>) {`,
      );
      lines.push(`      const { axiomQueryManager } = require('atmx-react');`);
      lines.push(
        `      const def = (this as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);`,
      );
      lines.push(`      axiomQueryManager.connect(def);`);
      lines.push(`    },`);
      lines.push(
        `    disconnect(methodName: string, args?: Record<string, any>) {`,
      );
      lines.push(`      const { axiomQueryManager } = require('atmx-react');`);
      lines.push(
        `      const def = (this as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);`,
      );
      lines.push(`      axiomQueryManager.disconnect(def);`);
      lines.push(`    },`);
      lines.push(
        `    send(methodName: string, payload: any, args?: Record<string, any>) {`,
      );
      lines.push(`      const { axiomQueryManager } = require('atmx-react');`);
      lines.push(
        `      const def = (this as any)[\`get\${methodName.charAt(0).toUpperCase() + methodName.slice(1)}Def\`](args);`,
      );
      lines.push(`      axiomQueryManager.send(def, payload);`);
      lines.push(`    }`);
    } else {
      lines.push(`    setAuthToken(methodName: string, token: string) {`);
      lines.push(
        `      (window as any).atmx?.setAuthToken("${ns}", methodName, token);`,
      );
      lines.push(`    },`);
      lines.push(`    clearAuthToken(methodName: string) {`);
      lines.push(
        `      (window as any).atmx?.clearAuthToken("${ns}", methodName);`,
      );
      lines.push(`    },`);
      lines.push(
        `    connect(methodName: string, args?: Record<string, any>) {`,
      );
      lines.push(
        `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';`,
      );
      lines.push(
        `      (window as any).atmx?.connect(\`${ns}.\${methodName}(\${argsStr})\`);`,
      );
      lines.push(`    },`);
      lines.push(
        `    disconnect(methodName: string, args?: Record<string, any>) {`,
      );
      lines.push(
        `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';`,
      );
      lines.push(
        `      (window as any).atmx?.disconnect(\`${ns}.\${methodName}(\${argsStr})\`);`,
      );
      lines.push(`    },`);
      lines.push(
        `    send(methodName: string, payload: any, args?: Record<string, any>) {`,
      );
      lines.push(
        `      const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';`,
      );
      lines.push(
        `      (window as any).atmx?.send(\`${ns}.\${methodName}(\${argsStr})\`, payload);`,
      );
      lines.push(`    }`);
    }
    lines.push(`  },`);

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
