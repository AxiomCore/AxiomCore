"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.generateModels = generateModels;
const utils_1 = require("./utils");
function generateModels(multiIr) {
    const sections = [
        `// GENERATED CODE – DO NOT EDIT.\n/* eslint-disable @typescript-eslint/no-explicit-any */\n`
    ];
    // 1. Generate flat exports (NO namespaces)
    for (const [ns, ir] of Object.entries(multiIr)) {
        const pascalNs = (0, utils_1.pascalCase)(ns);
        const enumsList = Array.isArray(ir.enums) ? ir.enums : Object.values(ir.enums || {});
        const modelsList = Array.isArray(ir.models) ? ir.models : Object.values(ir.models || {});
        enumsList.forEach((en) => sections.push(generateEnum(en, pascalNs)));
        modelsList.forEach((model) => sections.push(generateInterface(model, pascalNs)));
    }
    // 2. Generate nested Mappers object
    sections.push(generateMappers(multiIr));
    return sections.join('\n');
}
function generateEnum(en, ns) {
    const name = `${ns}${(0, utils_1.pascalCase)(en.name)}`;
    const values = en.values.map(v => `  ${(0, utils_1.pascalCase)(v)}: "${v}"`).join(',\n');
    return `
export const ${name} = {
${values}
} as const;

export type ${name} = typeof ${name}[keyof typeof ${name}];
`;
}
function generateInterface(model, ns) {
    const name = `${ns}${(0, utils_1.pascalCase)(model.name)}`;
    const fields = model.fields.map(f => {
        const type = (0, utils_1.mapTypeToTs)(f.typeRef, ns); // ✅ pass namespace
        return `  ${(0, utils_1.camelCase)(f.name)}${f.isOptional ? '?' : ''}: ${type};`;
    }).join('\n');
    return `
export interface ${name} {
${fields}
}
`;
}
function generateMappers(multiIr) {
    const lines = [`export const Mappers: Record<string, any> = {`];
    for (const [ns, ir] of Object.entries(multiIr)) {
        const camelNs = (0, utils_1.camelCase)(ns);
        const pascalNs = (0, utils_1.pascalCase)(ns);
        lines.push(`  ${camelNs}: {`);
        const modelsList = Array.isArray(ir.models) ? ir.models : Object.values(ir.models || {});
        modelsList.forEach((model) => {
            const name = (0, utils_1.pascalCase)(model.name);
            const fullType = `${pascalNs}${name}`; // no namespace now
            lines.push(`    ${name}: {\n      fromJson: (json: any): ${fullType} => ({`);
            model.fields.forEach((f) => {
                lines.push(`        ${(0, utils_1.camelCase)(f.name)}: ${generateJsonLogic(f.typeRef, `json["${f.name}"]`, f.isOptional, 'fromJson', camelNs)},`);
            });
            lines.push(`      }),\n      toJson: (obj: any): any => ({`);
            model.fields.forEach((f) => {
                lines.push(`        "${f.name}": ${generateJsonLogic(f.typeRef, `obj.${(0, utils_1.camelCase)(f.name)}`, f.isOptional, 'toJson', camelNs)},`);
            });
            lines.push(`      })\n    },`);
        });
        lines.push(`  },`);
    }
    lines.push(`};\n`);
    return lines.join('\n');
}
function generateJsonLogic(typeRef, access, isOpt, mode, ns) {
    const wrap = (logic) => isOpt ? `(${access} == null ? undefined : ${logic})` : logic;
    if (!typeRef || !typeRef.kind)
        return access;
    if (typeRef.kind === 'primitive') {
        if (typeRef.value === 'dateTime') {
            return mode === 'fromJson'
                ? wrap(`new Date(${access})`)
                : wrap(`${access}.toISOString()`);
        }
        if (typeRef.value === 'bytes') {
            return mode === 'fromJson'
                ? wrap(`new Uint8Array(${access})`)
                : wrap(`Array.from(${access})`);
        }
        return access;
    }
    if (typeRef.kind === 'named') {
        const name = (0, utils_1.pascalCase)(typeRef.value);
        return wrap(`(Mappers.${ns}["${name}"] ? Mappers.${ns}["${name}"].${mode}(${access}) : ${access})`);
    }
    if (typeRef.kind === 'list') {
        return wrap(`${access}.map((e: any) => ${generateJsonLogic(typeRef.value, 'e', false, mode, ns)})`);
    }
    return access;
}
