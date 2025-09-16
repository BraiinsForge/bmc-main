import IconBraiinsPool from './braiins-pool.svg';
import * as pb from '@/proto';
import { assertUnreachable } from '@/lib/ts';

export { IconBraiinsPool };

export interface AccountIconProps {
    type: pb.AccountType;
    size: number;

    className?: string;
    style?: CSSProperties;
}
export function AccountIcon(props: AccountIconProps) {
    const { type, size, style, className } = props;

    switch (type) {
        case pb.AccountType.UNSPECIFIED:
            return null;

        case pb.AccountType.BRAIINSPOOL:
            return <IconBraiinsPool width={size} style={style} className={className} />;

        default:
            assertUnreachable(type);
    }
}
