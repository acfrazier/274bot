/**
 * Ready BZip2 module stub (replaces #/io/BZip2.js which awaits wasm init at import).
 */
const BZip2 = {
    async init() {},
    decompress(_src: Uint8Array, _size?: number, _small?: boolean): Uint8Array {
        throw new Error('bzip2 stub: decompress should not run for pack .dat loads');
    },
    compress(_src: Uint8Array, _a?: boolean, _b?: boolean): Uint8Array {
        throw new Error('bzip2 stub: compress should not run offline');
    },
};
export default BZip2;
