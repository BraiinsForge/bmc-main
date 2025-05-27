import { PureComponent } from 'react';
import { Notifications as Component, type NotificationsProps, type NotificationItem } from './Notifications';
import { number, randomItem, arrayOf } from '@/mocks';

export default {
    title: 'components/Notifications',
    component: Component,
};

type TestState = {
    items: NotificationsProps['items'];
};
const content: string =
    'Lorem ipsum dolor sit amet, consectetur adipisicing elit. A accusantium ad adipisci consequatur.';
function btn(children: string, onClick: () => void, danger: boolean = false) {
    const style = { padding: '8px 16px' };
    if (danger) Object.assign(style, { background: 'rgb(150, 60, 60)', color: 'white' });

    return <button type="button" onClick={onClick} children={children} style={style} />;
}

class Test extends PureComponent<NotificationsProps, TestState> {
    readonly state: TestState = {
        items: [
            { id: 0, kind: 'info', content, title: 'Title' },
            { id: 1, kind: 'error', content },
            { id: 2, kind: 'warning', content },
        ],
    };

    #i = -1;
    #push = (count: number = 1): void => {
        const newItems = arrayOf(count, () => {
            const id = this.#i--;
            return {
                id,
                counter: number(0, 15, false),
                kind: randomItem<NotificationItem['kind']>(['info', 'warning', 'error', 'success']),
                title: `Title #${id}`,
                content: `Caption #${id}`,
            } satisfies NotificationItem;
        });
        this.setState({ items: [...this.state.items, ...newItems] });
    };
    #handleRemove = ({ id }: { id: StrNum }): void => {
        this.setState(s => ({ items: s.items.filter(x => x.id !== id) }));
    };
    #filter = (fn: (d: NotificationItem, i: number, a: ReadonlyArray<NotificationItem>) => boolean): void => {
        this.setState(s => ({ items: s.items.filter(fn) }));
    };

    render() {
        const handleClear = () => this.#filter(() => false);

        return (
            <div>
                <div style={{ padding: 16 }}>
                    {btn('Add', () => this.#push(1))}
                    {btn('Add 10', () => this.#push(10))}
                    <br />
                    {btn('Remove 1st', () => this.#filter((_, i) => i !== 0), true)}
                    {btn('Remove last', () => this.#filter((_, i, a) => i !== a.length - 1), true)}
                    {btn('Remove Odd', () => this.#filter((_, i) => i % 2 === 0), true)}
                    {btn('Remove Even', () => this.#filter((_, i) => i % 2 !== 0), true)}
                    {btn('Remove All', handleClear, true)}

                    <pre
                        style={{
                            backgroundColor: '#ccc',
                            marginTop: 16,
                            padding: 8,
                            maxWidth: 500,
                            overflow: 'auto',
                        }}
                        children={JSON.stringify(this.state.items, null, 2)}
                    />
                </div>

                <Component {...this.props} items={this.state.items} onHide={this.#handleRemove} onClear={handleClear} />
            </div>
        );
    }
}
export const Notifications = (args: NotificationsProps) => <Test {...args} />;
Notifications.args = { top: 0 } as NotificationsProps;
