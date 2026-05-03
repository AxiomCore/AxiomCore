"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.generateSdk = generateSdk;
const utils_1 = require("./utils");
function generateSdk(multiIr, isReact = false) {
    const lines = [
        `// GENERATED CODE – DO NOT EDIT.`,
        `import * as models from './models';\n`,
    ];
    if (isReact) {
        lines.push(`import { useAxiomQuery, useAxiomMutation } from 'atmx-react';`);
        lines.push(`import type { AxiomQueryDef } from 'atmx-react';\n`);
    }
    for (const [ns, ir] of Object.entries(multiIr)) {
        const pascalNs = (0, utils_1.pascalCase)(ns);
        lines.push(`export class ${pascalNs}Module {`);
        const endpointsMap = ir.endpoints || {};
        const endpoints = Array.isArray(endpointsMap)
            ? endpointsMap
            : Object.values(endpointsMap);
        endpoints.forEach((ep) => {
            lines.push(generateEndpointMethod(ep, ns, pascalNs, isReact));
        });
        lines.push(`}\n`);
    }
    lines.push(`export class AxiomSdk {`);
    for (const ns of Object.keys(multiIr)) {
        lines.push(`  public readonly ${(0, utils_1.camelCase)(ns)} = new ${(0, utils_1.pascalCase)(ns)}Module();`);
    }
    lines.push(`}\nexport const sdk = new AxiomSdk();`);
    return lines.join("\n");
}
function generateEndpointMethod(ep, ns, pascalNs, isReact) {
    const rawParams = ep.parameters || [];
    const params = Array.isArray(rawParams)
        ? rawParams
        : Object.values(rawParams);
    const argType = params.length > 0
        ? `{ ${params.map((p) => `${(0, utils_1.camelCase)(p.name)}${p.isOptional ? "?" : ""}: ${prefixModels((0, utils_1.mapTypeToTs)(p.typeRef, pascalNs))}`).join(", ")} }`
        : "void";
    const isQuery = ep.method ? ep.method.toUpperCase() === "GET" : true;
    const rawReturnType = (0, utils_1.mapTypeToTs)(ep.returnType, pascalNs);
    const returnType = rawReturnType === "void" || rawReturnType === "any"
        ? rawReturnType
        : prefixModels(rawReturnType);
    if (isReact) {
        const bodyParam = params.find((p) => p.source === "body");
        const payloadLogic = bodyParam
            ? `const payload = (args as any)?.${(0, utils_1.camelCase)(bodyParam.name)};`
            : `const payload = undefined;`;
        const decLogic = generateLambda(ep.returnType, "fromJson", pascalNs);
        const serLogic = bodyParam
            ? generateLambda(bodyParam.typeRef, "toJson", pascalNs)
            : `(p: any) => p`;
        return `
  get${(0, utils_1.pascalCase)(ep.name)}Def(args${params.length > 0 ? "?" : ""}: ${argType === "void" ? "any" : argType}): AxiomQueryDef<${returnType}> {
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
  }

  use${(0, utils_1.pascalCase)(ep.name)}${!isQuery ? "Mutation" : ""}(${isQuery ? `args${params.length > 0 ? "?" : ""}: ${argType === "void" ? "any" : argType}, options?: { enabled?: boolean }` : ""}) {
    ${isQuery
            ? `return useAxiomQuery<${returnType}>(this.get${(0, utils_1.pascalCase)(ep.name)}Def(args), options);`
            : `return useAxiomMutation<${returnType}, ${argType === "void" ? "void | Record<string,any>" : argType}>((args) => this.get${(0, utils_1.pascalCase)(ep.name)}Def(args));`}
  }\n`;
    }
    else {
        return `
  ${(0, utils_1.camelCase)(ep.name)}(args${params.length > 0 ? "?" : ""}: ${argType === "void" ? "any" : argType}): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return \`${ns}.${ep.name}(\${argsStr})\`;
  }\n`;
    }
}
function generateLambda(typeRef, mode, ns) {
    if (!typeRef || !typeRef.kind || typeRef.kind === "void")
        return mode === "fromJson" ? `() => undefined` : `(p: any) => p`;
    if (typeRef.kind === "list" && typeRef.value?.kind === "named")
        return `(data: any[]) => data.map(models.Mappers.${(0, utils_1.camelCase)(ns)}.${(0, utils_1.pascalCase)(typeRef.value.value)}.${mode})`;
    if (typeRef.kind === "named")
        return `models.Mappers.${(0, utils_1.camelCase)(ns)}.${(0, utils_1.pascalCase)(typeRef.value)}.${mode}`;
    return `(data: any) => data`;
}
function prefixModels(type) {
    const primitives = [
        "string",
        "number",
        "boolean",
        "Date",
        "Uint8Array",
        "void",
        "any",
    ];
    if (!type || primitives.includes(type))
        return type;
    if (type.endsWith("[]"))
        return `${prefixModels(type.slice(0, -2))}[]`;
    if (type.startsWith("models."))
        return type;
    return `models.${type}`;
}
