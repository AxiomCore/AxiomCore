"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.generateSdk = generateSdk;
const utils_1 = require("./utils");
function generateSdk(multiIr) {
    const lines = [
        `// GENERATED CODE – DO NOT EDIT.`,
        `import * as models from './models';\n`,
    ];
    for (const [ns, ir] of Object.entries(multiIr)) {
        const pascalNs = (0, utils_1.pascalCase)(ns);
        lines.push(`export class ${pascalNs}Module {`);
        // Support both old array format and new object map format
        const endpointsMap = ir.endpoints || {};
        const endpoints = Array.isArray(endpointsMap)
            ? endpointsMap
            : Object.values(endpointsMap);
        endpoints.forEach((ep) => {
            lines.push(generateEndpointMethod(ep, ns, pascalNs));
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
function generateEndpointMethod(ep, ns, pascalNs) {
    const params = ep.parameters || [];
    const argType = params.length > 0
        ? `{ ${params.map((p) => `${(0, utils_1.camelCase)(p.name)}${p.isOptional ? "?" : ""}: ${prefixModels((0, utils_1.mapTypeToTs)(p.typeRef, pascalNs))}`).join(", ")} }`
        : "void";
    // We generate a method that takes the typed arguments and returns the exact string ATMX expects
    return `
  /** RPC String Generator for <AxQuery> or <AxMutate> */
  ${(0, utils_1.camelCase)(ep.name)}(args${params.length > 0 ? "" : "?"}: ${argType}): string {
    const argsStr = args && Object.keys(args).length > 0 ? JSON.stringify(args) : '';
    return \`${ns}.${ep.name}(\${argsStr})\`;
  }\n`;
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
