import { InlineLoading as Upstream, type InlineLoadingProps as UpstreamProps } from '@carbon/react';
import cn from 'clsx';

export type InlineLoadingProps = {
    description?: UpstreamProps['description'];
    iconDescription?: UpstreamProps['iconDescription'];
    status: NonNullable<UpstreamProps['status']> | ReactElement;

    className?: string;
    style?: CSSProperties;
};

export function InlineLoading({ status, iconDescription, description, className, style }: InlineLoadingProps) {
    if (typeof status === 'string') {
        return <Upstream status={status} iconDescription={iconDescription} description={description} />;
    }
    return (
        <div aria-live="assertive" className={cn('cds--inline-loading', className)} style={style}>
            <div className="cds--inline-loading__animation" children={status} />
            <div className="cds--inline-loading__text" children={description} />
        </div>
    );
}
