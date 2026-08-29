// FILE: atmx-cli/src/generators/model-generator.ts
import { AxiomDomainModel, AxiomEnum, AxiomModel, MultiIR } from "../types";
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
    sections.push(generateDomainProjectionTypes(ir.domain, modelsList));

    sections.push(`}\n`);
  }

  sections.push(generateMappers(multiIr));
  return sections.join("\n");
}

function generateDomainProjectionTypes(domain: AxiomDomainModel | undefined, models: any[]): string {
  if (!domain?.projections || !domain?.entities) return "";
  const entities = domain.entities || {};
  const projections = domain.projections || {};
  const availableModels = new Set(models.map((model) => String(model?.name || "")));
  const lines = ["  export namespace Domain {"];
  let count = 0;
  for (const [name, projection] of Object.entries(projections)) {
    const entity = entities[(projection as any).entity];
    const model = entity?.model;
    const fields = Array.isArray((projection as any).fields) ? (projection as any).fields : [];
    if (!model || !availableModels.has(model) || fields.length === 0) continue;
    const fieldUnion = fields.map((field: string) => JSON.stringify(camelCase(field))).join(" | ");
    lines.push(`    export type ${pascalCase(name)} = Pick<${pascalCase(model)}, ${fieldUnion}>;`);
    count += 1;
  }
  lines.push("  }");
  return count ? lines.join("\n") : "";
}

function generateEnum(en: AxiomEnum): string {
  const name = pascalCase(en.name);
  const values = en.values.map((v) => `  ${pascalCase(v)}: "${v}"`).join(",\n");
  return [
    "",
    `  export const ${name} = {`,
    values,
    "  } as const;",
    `  export type ${name} = typeof ${name}[keyof typeof ${name}];`,
    "",
  ].join("\n");
}

function generateInterface(model: AxiomModel, ns: string): string {
  const name = pascalCase(model.name);
  const fields = model.fields
    .map((f) => {
      const type = mapTypeToTs(f.typeRef, ns);
      return `    ${camelCase(f.name)}${f.isOptional ? "?" : ""}: ${type};`;
    })
    .join("\n");

  return ["", `  export interface ${name} {`, fields, "  }", ""].join("\n");
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
