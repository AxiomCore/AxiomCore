import type { LanguageRegistration } from '@shikijs/types';

/**
 * TextMate grammar for documentation examples. Keep keywords synchronized with
 * `acore/src/lexer.rs` and the Acore UI declarations parsed by `axiom-ui`.
 */
export const acoreLanguage: LanguageRegistration = {
  name: 'acore',
  displayName: 'Acore',
  scopeName: 'source.acore',
  fileTypes: ['acore'],
  patterns: [
    { include: '#comments' },
    { include: '#strings' },
    { include: '#numbers' },
    { include: '#constants' },
    { include: '#annotations' },
    { include: '#declarations' },
    { include: '#keywords' },
    { include: '#types' },
    { include: '#operators' },
  ],
  repository: {
    comments: {
      patterns: [
        { name: 'comment.line.double-slash.acore', match: '//.*$' },
      ],
    },
    strings: {
      patterns: [
        {
          name: 'string.interpolated.acore',
          begin: '\\$"',
          end: '"',
          patterns: [
            { name: 'constant.character.escape.acore', match: '\\\\.' },
            { name: 'meta.interpolation.acore', begin: '(?<!\\{)\\{(?!\\{)', end: '\\}', patterns: [{ include: '$self' }] },
          ],
        },
        {
          name: 'string.quoted.triple.acore',
          begin: '"""',
          end: '"""',
          patterns: [{ include: '#escapes' }],
        },
        {
          name: 'string.quoted.double.acore',
          begin: '"',
          end: '"',
          patterns: [{ include: '#escapes' }],
        },
        {
          name: 'string.quoted.raw.acore',
          begin: '#"',
          end: '"#',
        },
      ],
    },
    escapes: {
      patterns: [
        { name: 'constant.character.escape.acore', match: '\\\\.' },
        {
          name: 'meta.interpolation.acore',
          begin: '\\\\?\\(',
          end: '\\)',
          patterns: [{ include: '$self' }],
        },
      ],
    },
    numbers: {
      patterns: [
        { name: 'constant.numeric.duration.acore', match: '\\b\\d+(?:ms|s|m|h|d)\\b' },
        { name: 'constant.numeric.float.acore', match: '\\b-?\\d+\\.\\d+\\b' },
        { name: 'constant.numeric.integer.acore', match: '\\b-?\\d+\\b' },
      ],
    },
    constants: {
      patterns: [
        { name: 'constant.language.acore', match: '\\b(?:true|false|null)\\b' },
      ],
    },
    annotations: {
      patterns: [
        { name: 'storage.type.annotation.acore', match: '@[A-Za-z_][A-Za-z0-9_]*' },
      ],
    },
    declarations: {
      patterns: [
        {
          match: '\\b(pure)\\s+(fn)\\s+([A-Za-z_][A-Za-z0-9_]*)',
          captures: {
            1: { name: 'storage.modifier.acore' },
            2: { name: 'keyword.declaration.acore' },
            3: { name: 'entity.name.function.acore' },
          },
        },
        {
          match: '\\b(module|class|typealias|function|app|page|state|action|query|mutation|model|endpoint|entity|projection|variant|policy|relationship|invariant|failure|validator|resource|operation|permission|guard|configuration|secret|serverPolicy|operationPolicy|interceptor|service|group|database|schema|table|index|trigger|form_model|store|transaction)\\s+([A-Za-z_][A-Za-z0-9_.]*)',
          captures: {
            1: { name: 'keyword.declaration.acore' },
            2: { name: 'entity.name.type.acore' },
          },
        },
      ],
    },
    keywords: {
      patterns: [
        {
          name: 'keyword.control.acore',
          match: '\\b(?:if|else|for|when|let|throw|trace|is|as|await|return|fail|match|case|try|catch|finally|managed|perform|invoke|result|parallel|enqueue)\\b',
        },
        {
          name: 'keyword.other.acore',
          match: '\\b(?:import|use|contract|theme|component|from|export|audience|appearance|read|amends|extend|extends|external|abstract|open|local|hidden|const|fixed|this|outer|super|view|route|on_press|profile|backend|override|also|entity)\\b',
        },
      ],
    },
    types: {
      patterns: [
        {
          name: 'storage.type.acore',
          match: '\\b(?:Any|Null|Boolean|Bool|Int|Int32|Int64|Float|Float32|Float64|Number|String|Bytes|DateTime|Json|Void|Optional|Duration|DataSize|List|Listing|Set|Map|Mapping|Object|Dynamic|Module|Class|Function)\\b',
        },
        {
          name: 'entity.name.type.acore',
          match: '\\b[A-Z][A-Za-z0-9_]*\\b',
        },
      ],
    },
    operators: {
      patterns: [
        {
          name: 'keyword.operator.acore',
          match: '\\?\\.|\\?\\?|!!|\\|>|->|\\*\\*|~/|&&|\\|\\||==|!=|<=|>=|[=+\\-*/%<>!?:|]',
        },
      ],
    },
  },
};
