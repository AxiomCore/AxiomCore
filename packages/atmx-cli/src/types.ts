export interface AxiomIR {
  serviceName: string;
  endpoints: AxiomEndpoint[];
  models: Record<string, AxiomModel>;
  enums: Record<string, AxiomEnum>;
  domain?: AxiomDomainModel;
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
  requestProjection?: string;
  responseProjection?: string;
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

export interface AxiomDomainModel {
  entities: Record<string, AxiomDomainEntity>;
  projections: Record<string, AxiomDomainProjection>;
}

export interface AxiomDomainEntity {
  id?: string;
  model: string;
  key: string[];
}

export interface AxiomDomainProjection {
  id?: string;
  entity: string;
  fields: string[];
  audience: string;
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
