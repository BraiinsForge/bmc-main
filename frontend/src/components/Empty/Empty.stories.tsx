import { action } from 'storybook/actions';

import { Button } from '@/components';
import { Empty as Component, type EmptyProps } from './Empty';
import { CloudMonitoring, QuestionAnswering } from '@carbon/react/icons';

export default {
    title: 'components/Empty',
    component: Component,
};

export function Empty(args: EmptyProps) {
    return (
        <div style={{ maxWidth: '600px' }}>
            <Component {...args} />
        </div>
    );
}

Empty.args = {
    icon: CloudMonitoring,
    title: 'There are no workers to monitor…',
    message: (
        <span>
            You must first connect workers and then you&apos;ll be able to see a summary here of the events that were
            recorded by our monitoring system.
        </span>
    ),
    controls: (
        <Button id="connect-workers" icon={QuestionAnswering} children="Connect Workers" onClick={action('onClick')} />
    ),
} as EmptyProps;
