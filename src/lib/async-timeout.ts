import { t } from "../i18n/translate";

export function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  message = t("lib.errors.timedOut"),
): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(message)), timeoutMs);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error: unknown) => {
        clearTimeout(timer);
        reject(error);
      },
    );
  });
}
