"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.generateModels = generateModels;
const utils_1 = require("./utils");
function generateModels(multiIr) {
    const sections = [
        `// GENERATED CODE – DO NOT EDIT.\n/* eslint-disable @typescript-eslint/no-explicit-any */\n`,
        `/* eslint-disable @typescript-eslint/no-namespace */\n`, // ✨ FIX: Disable namespace lint error
    ];
    for (const [ns, ir] of Object.entries(multiIr)) {
        const camelNs = (0, utils_1.camelCase)(ns);
        // ✨ FIX: Use proper TS namespaces
        sections.push(`export namespace ${camelNs} {`);
        const enumsList = Array.isArray(ir.enums)
            ? ir.enums
            : Object.values(ir.enums || {});
        const modelsList = Array.isArray(ir.models)
            ? ir.models
            : Object.values(ir.models || {});
        enumsList.forEach((en) => sections.push(generateEnum(en)));
        modelsList.forEach((model) => sections.push(generateInterface(model, camelNs)));
        sections.push(generateDomainProjectionTypes(ir.domain, modelsList));
        sections.push(`}\n`);
    }
    sections.push(generateMappers(multiIr));
    return sections.join("\n");
}
function generateDomainProjectionTypes(domain, models) {
    if (!domain?.projections || !domain?.entities)
        return "";
    const entities = domain.entities || {};
    const projections = domain.projections || {};
    const availableModels = new Set(models.map((model) => String(model?.name || "")));
    const lines = ["  export namespace Domain {"];
    let count = 0;
    for (const [name, projection] of Object.entries(projections)) {
        const entity = entities[projection.entity];
        const model = entity?.model;
        const fields = Array.isArray(projection.fields) ? projection.fields : [];
        if (!model || !availableModels.has(model) || fields.length === 0)
            continue;
        const fieldUnion = fields.map((field) => JSON.stringify((0, utils_1.camelCase)(field))).join(" | ");
        lines.push(`    export type ${(0, utils_1.pascalCase)(name)} = Pick<${(0, utils_1.pascalCase)(model)}, ${fieldUnion}>;`);
        count += 1;
    }
    lines.push("  }");
    return count ? lines.join("\n") : "";
}
function generateEnum(en) {
    const name = (0, utils_1.pascalCase)(en.name);
    const values = en.values.map((v) => `  ${(0, utils_1.pascalCase)(v)}: "${v}"`).join(",\n");
    return [
        "",
        `  export const ${name} = {`,
        values,
        "  } as const;",
        `  export type ${name} = typeof ${name}[keyof typeof ${name}];`,
        "",
    ].join("\n");
}
function generateInterface(model, ns) {
    const name = (0, utils_1.pascalCase)(model.name);
    const fields = model.fields
        .map((f) => {
        const type = (0, utils_1.mapTypeToTs)(f.typeRef, ns);
        return `    ${(0, utils_1.camelCase)(f.name)}${f.isOptional ? "?" : ""}: ${type};`;
    })
        .join("\n");
    return ["", `  export interface ${name} {`, fields, "  }", ""].join("\n");
}
function generateMappers(multiIr) {
    const lines = [`export const Mappers: Record<string, any> = {`];
    for (const [ns, ir] of Object.entries(multiIr)) {
        const camelNs = (0, utils_1.camelCase)(ns);
        lines.push(`  ${camelNs}: {`);
        const modelsList = Array.isArray(ir.models)
            ? ir.models
            : Object.values(ir.models || {});
        modelsList.forEach((model) => {
            const name = (0, utils_1.pascalCase)(model.name);
            const fullType = `${camelNs}.${name}`;
            lines.push(`    ${name}: {\n      fromJson: (json: any): ${fullType} => ({`);
            model.fields.forEach((f) => {
                lines.push(`        ${(0, utils_1.camelCase)(f.name)}: ${generateJsonLogic(f.typeRef, `json["${f.name}"]`, f.isOptional, "fromJson", camelNs)},`);
            });
            lines.push(`      }),\n      toJson: (obj: any): any => ({`);
            model.fields.forEach((f) => {
                lines.push(`        "${f.name}": ${generateJsonLogic(f.typeRef, `obj.${(0, utils_1.camelCase)(f.name)}`, f.isOptional, "toJson", camelNs)},`);
            });
            lines.push(`      })\n    },`);
        });
        lines.push(`  },`);
    }
    lines.push(`};\n`);
    return lines.join("\n");
}
function generateJsonLogic(typeRef, access, isOpt, mode, ns) {
    const wrap = (logic) => isOpt ? `(${access} == null ? undefined : ${logic})` : logic;
    if (!typeRef || !typeRef.kind)
        return access;
    if (typeRef.kind === "dateTime")
        return mode === "fromJson"
            ? wrap(`new Date(${access})`)
            : wrap(`${access}.toISOString()`);
    if (typeRef.kind === "bytes")
        return mode === "fromJson"
            ? wrap(`new Uint8Array(${access})`)
            : wrap(`Array.from(${access})`);
    if (typeRef.kind === "named") {
        const name = (0, utils_1.pascalCase)(typeRef.value);
        return wrap(`(Mappers.${ns}["${name}"] ? Mappers.${ns}["${name}"].${mode}(${access}) : ${access})`);
    }
    if (typeRef.kind === "list")
        return wrap(`${access}.map((e: any) => ${generateJsonLogic(typeRef.value, "e", false, mode, ns)})`);
    return access;
}
