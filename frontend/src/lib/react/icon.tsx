import type { ElementType, FunctionComponent } from 'react';
import { isValidElementType } from 'react-is';

import type { CarbonIconProps, CarbonIconType } from '@carbon/react/icons';

export function carbonizeSvgIcon(
    svg: ElementType | { __esModule: true; default: FunctionComponent },
    displayName: string,
): CarbonIconType {
    const SvgComponent: ElementType = isValidElementType(svg) ? svg : svg.default;

    const res = ((props: CarbonIconProps) => {
        const { size = 16, fill = 'currentColor', ...rest } = props;
        return <SvgComponent width={size} height={size} style={{ fill }} {...rest} />;
    }) as CarbonIconType;
    res.displayName = displayName;

    return res;
}

export type { CarbonIconType, CarbonIconProps };
