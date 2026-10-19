import {
  op_crypto_get_random_values,
  op_crypto_random_uuid,
} from "ext:core/ops";

globalThis.crypto = {
  randomUUID: () => op_crypto_random_uuid(),
  getRandomValues: (typedArray) =>
    op_crypto_get_random_values(typedArray, typedArray),
};
