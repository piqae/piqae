import type { components } from '../src/generated/schema.js';
import type {
  Agent,
  ApiKey,
  BillingSummary,
  CreateApiKey,
  CreateJob,
  CreateStock,
  CreateTarget,
  CreateTargetBinding,
  CreateUpload,
  DeploymentMeta,
  Health,
  Job,
  JobEvent,
  JobOptions,
  JobState,
  PlatformAccount,
  Printer,
  Stock,
  Target,
  TargetBinding,
  TargetReadiness,
  Upload,
  UsageSummary,
  Webhook,
  WebhookDelivery,
  Workspace,
  WorkspaceMember
} from '../src/types.js';

type Schemas = components['schemas'];
type Assert<T extends true> = T;
type Assignable<From, To> = [From] extends [To] ? true : false;

// Request types must never allow a payload that the canonical OpenAPI schema rejects.
type _CreateApiKey = Assert<Assignable<CreateApiKey, Schemas['CreateApiKey']>>;
type _CreateJob = Assert<Assignable<CreateJob, Schemas['CreateJob']>>;
type _CreateStock = Assert<Assignable<CreateStock, Schemas['CreateStock']>>;
type _CreateTarget = Assert<Assignable<CreateTarget, Schemas['CreateTarget']>>;
type _CreateTargetBinding = Assert<
  Assignable<CreateTargetBinding, Schemas['CreateTargetBinding']>
>;
type _CreateUpload = Assert<Assignable<CreateUpload, Schemas['CreateUpload']>>;
type _JobOptions = Assert<Assignable<JobOptions, Schemas['JobOptions']>>;

// Every documented response must be safely representable by the ergonomic SDK type.
type _Agent = Assert<Assignable<Schemas['Agent'], Agent>>;
type _ApiKey = Assert<Assignable<Schemas['ApiKey'], ApiKey>>;
type _BillingSummary = Assert<Assignable<Schemas['BillingSummary'], BillingSummary>>;
type _DeploymentMeta = Assert<Assignable<Schemas['DeploymentMeta'], DeploymentMeta>>;
type _Health = Assert<Assignable<Schemas['Health'], Health>>;
type _Job = Assert<Assignable<Schemas['Job'], Job>>;
type _JobEvent = Assert<Assignable<Schemas['JobEvent'], JobEvent>>;
type _JobState = Assert<Assignable<Schemas['JobState'], JobState>>;
type _PlatformAccount = Assert<Assignable<Schemas['PlatformAccount'], PlatformAccount>>;
type _Printer = Assert<Assignable<Schemas['Printer'], Printer>>;
type _Stock = Assert<Assignable<Schemas['Stock'], Stock>>;
type _Target = Assert<Assignable<Schemas['Target'], Target>>;
type _TargetBinding = Assert<Assignable<Schemas['TargetBinding'], TargetBinding>>;
type _TargetReadiness = Assert<Assignable<Schemas['TargetReadiness'], TargetReadiness>>;
type _Upload = Assert<Assignable<Schemas['Upload'], Upload>>;
type _UsageSummary = Assert<Assignable<Schemas['UsageSummary'], UsageSummary>>;
type _Webhook = Assert<Assignable<Schemas['Webhook'], Webhook>>;
type _WebhookDelivery = Assert<Assignable<Schemas['WebhookDelivery'], WebhookDelivery>>;
type _Workspace = Assert<Assignable<Schemas['Workspace'], Workspace>>;
type _WorkspaceMember = Assert<Assignable<Schemas['WorkspaceMember'], WorkspaceMember>>;
