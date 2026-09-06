import { createRequire } from 'node:module';
import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const repositoryRoot = resolve(import.meta.dirname, '..');
const webRoot = resolve(repositoryRoot, 'concord/web');
const requireFromWeb = createRequire(resolve(webRoot, 'package.json'));
const { compile } = await import(requireFromWeb.resolve('json-schema-to-typescript'));

const [schemaPath, outputDirectory] = process.argv.slice(2);
if (!schemaPath || !outputDirectory) {
  throw new Error('usage: node scripts/generate-contract.mjs <schema.json> <output-directory>');
}

const schemaText = await readFile(schemaPath, 'utf8');
const schema = JSON.parse(schemaText);
const Ajv2020 = requireFromWeb('ajv/dist/2020').default;
const addFormats = requireFromWeb('ajv-formats').default;
const ajv = new Ajv2020({ allErrors: true, strict: true });
configureFormats(ajv, addFormats);
const definitions = schema.$defs;
const clientValidator = ajv.compile({ ...schema.properties.client_message, $defs: definitions });
const serverValidator = ajv.compile({ ...schema.properties.server_event, $defs: definitions });
if (clientValidator({ type: 'unknown_command' })) {
  throw new Error('generated client validator accepted an unknown command');
}
if (!serverValidator({ type: 'error', code: 'INVALID_INPUT', message: 'invalid message' })) {
  throw new Error(`generated server validator rejected a valid error event: ${ajv.errorsText(serverValidator.errors)}`);
}
if (serverValidator({ type: 'error', code: 7, message: false })) {
  throw new Error('generated server validator accepted invalid error field types');
}
if (serverValidator({ type: 'server_limits', max_message_length: 9007199254740992, max_file_size_mb: 1 })) {
  throw new Error('generated server validator accepted an integer outside the JavaScript safe range');
}
const types = await compile(schema, 'ConcordWebSocketContract', {
  bannerComment: '// Generated from the production Rust Serde DTOs. Do not edit.\n',
  additionalProperties: false,
  unknownAny: true,
  unreachableDefinitions: true,
});

const validator = `// Generated from the production Rust Serde DTOs. Do not edit.
import Ajv2020 from 'ajv/dist/2020';
import addFormats from 'ajv-formats';
import type { ConcordWebSocketContract } from './contract';

const rootSchema = ${JSON.stringify(schema, null, 2)} as const;
const ajv = new Ajv2020({ allErrors: true, strict: true });
addFormats(ajv);
ajv.addFormat('int32', { type: 'number', validate: (value: number) => Number.isInteger(value) && value >= -2147483648 && value <= 2147483647 });
ajv.addFormat('int64', { type: 'number', validate: Number.isSafeInteger });
ajv.addFormat('uint', { type: 'number', validate: (value: number) => Number.isSafeInteger(value) && value >= 0 });
ajv.addFormat('uint32', { type: 'number', validate: (value: number) => Number.isInteger(value) && value >= 0 && value <= 4294967295 });
ajv.addFormat('uint64', { type: 'number', validate: (value: number) => Number.isSafeInteger(value) && value >= 0 });
const definitions = rootSchema.$defs;
const clientSchema = { ...rootSchema.properties.client_message, $defs: definitions };
const serverSchema = { ...rootSchema.properties.server_event, $defs: definitions };
const validateClient = ajv.compile(clientSchema);
const validateServer = ajv.compile(serverSchema);

export function isClientMessage(value: unknown): value is ConcordWebSocketContract['client_message'] {
  return validateClient(value);
}

export function isServerEvent(value: unknown): value is ConcordWebSocketContract['server_event'] {
  return validateServer(value);
}
`;

await Promise.all([
  writeFile(resolve(outputDirectory, 'contract.ts'), types),
  writeFile(resolve(outputDirectory, 'validator.ts'), validator),
]);

function configureFormats(validator, installStandardFormats) {
  installStandardFormats(validator);
  validator.addFormat('int32', { type: 'number', validate: (value) => Number.isInteger(value) && value >= -2147483648 && value <= 2147483647 });
  validator.addFormat('int64', { type: 'number', validate: Number.isSafeInteger });
  validator.addFormat('uint', { type: 'number', validate: (value) => Number.isSafeInteger(value) && value >= 0 });
  validator.addFormat('uint32', { type: 'number', validate: (value) => Number.isInteger(value) && value >= 0 && value <= 4294967295 });
  validator.addFormat('uint64', { type: 'number', validate: (value) => Number.isSafeInteger(value) && value >= 0 });
}
