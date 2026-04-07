import type { ImgHTMLAttributes } from 'react';
import type * as pb from '@/proto';

import * as Icons from '@/components/images/icons';

export interface ScenePreviewProps extends ImgHTMLAttributes<HTMLImageElement> {
    kind: Maybe<'combined' | { manifest?: pb.WidgetManifest }>;
}
export function ScenePreview(props: ScenePreviewProps) {
    const { kind } = props;
    if (kind == null) return null;

    if (kind === 'combined') return <Icons.WidgetCombined size={40} />;

    return <Icons.WidgetCombined size={40} />;
}
