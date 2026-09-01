export declare const ADMIN_EXTENSION_ORIGIN: string;
export declare const SHOPIFY_ADMIN_ORIGIN: string;
export declare function adminExtensionCors(
  response: Response,
  request?: Request,
): Response;
export declare function isAdminExtensionPrintSourcePath(
  pathname: string,
): boolean;
export declare function isAdminExtensionPreflightPath(
  pathname: string,
): boolean;
export declare function adminExtensionPreflight(
  request: Request,
): Response | null;
export declare function adminExtensionPreflightMiddleware(
  request: {
    method: string;
    path: string;
    originalUrl: string;
    headers: Record<string, string | string[] | undefined>;
  },
  response: {
    status(status: number): unknown;
    setHeader(name: string, value: string): unknown;
    send(body: string): unknown;
    end(): unknown;
  },
  next: (error?: unknown) => void,
): Promise<void>;
