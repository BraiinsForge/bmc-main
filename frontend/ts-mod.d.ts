declare module '*.svg' {
    import type { SVGProps, FunctionComponent } from 'react';
    const Component: FunctionComponent<SVGProps<SVGSVGElement>>;
    export default Component;
}

/* these are taken from "@rsbuild/core/types" */

// Images
declare module '*.jpg' {
    const src: string;
    export default src;
}
declare module '*.jpeg' {
    const src: string;
    export default src;
}
declare module '*.png' {
    const src: string;
    export default src;
}
declare module '*.ico' {
    const src: string;
    export default src;
}

// Fonts
declare module '*.woff' {
    const src: string;
    export default src;
}
declare module '*.woff2' {
    const src: string;
    export default src;
}
declare module '*.eot' {
    const src: string;
    export default src;
}
declare module '*.ttf' {
    const src: string;
    export default src;
}
declare module '*.otf' {
    const src: string;
    export default src;
}

// CSS Modules
declare module '*.scss' {
    const classes: Readonly<Record<string, string>>;
    export default classes;
}
