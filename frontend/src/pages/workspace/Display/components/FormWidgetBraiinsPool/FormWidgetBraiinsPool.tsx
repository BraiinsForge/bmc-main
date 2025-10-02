import { useMemo, useCallback } from 'react';
import { type IntlShape, useIntl } from 'react-intl';

// App
import * as pb from '@/proto';
import { getID } from '../const';
import { Form, type iField, type FormPropsToValuesRec } from '@/lib/form';

// Components
import { ModalCustom, InlineNotification, AccountIcon, Datetime } from '@/components';
import {
    WidgetSizeSelector,
    type WidgetSizeSelectorProps,
    CheckYourScreenForPreview,
    BoundRadioGroup,
    type OptionItem,
    BoundDropdown,
} from '../shared';
import { CalendarAdd as IconCalendar } from '@carbon/react/icons';

// styles
import css from '../shared.scss';

const $ = getID('braiins-pool-form').get;

export interface FormWidgetBraiinsPoolProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];

    accountId: iField<pb.Account['id']> & { options: Array<pb.Account> };
    sceneStyle: iField<pb.BraiinsPoolWidget['braiinsPoolStyle']>;
    timeFrame: iField<pb.BraiinsPoolWidget['timeFrame']>;

    style?: CSSProperties;
}

function useOptions<T extends string | number>(
    values: Array<T>,
    stringifier: (intl: IntlShape, value: T) => null | string,
): Array<OptionItem<T>> {
    const intl = useIntl();
    return useMemo(() => {
        return values.map(x => ({
            value: x,
            label: String(stringifier(intl, x)),
        }));
    }, [intl, stringifier, values]);
}

function AccountElement(props: pb.Account) {
    const { accountType, accountName, createdAt } = props;

    return (
        <div className={css.accountElement}>
            <div className={css.accountElementName}>
                <AccountIcon type={accountType} size={18} />
                <span children={accountName} />
            </div>

            <div className={css.accountElementDate}>
                <IconCalendar size={16} />
                <Datetime value={createdAt} />
            </div>
        </div>
    );
}

export function FormWidgetBraiinsPool(props: FormWidgetBraiinsPoolProps) {
    const intl = useIntl();
    const { formatMessage } = intl;

    const {
        isOpen,
        isEdit,
        onClose,
        error,

        // Main
        widgetSize,
        accountId,
        sceneStyle,
        timeFrame,

        style,
    } = props;

    const txt = {
        braiinsPool: formatMessage({ defaultMessage: 'Braiins Pool' }),
        addScene: formatMessage({ defaultMessage: 'Add Scene' }),
        editScene: formatMessage({ defaultMessage: 'Edit Scene' }),
    };

    const sceneStyles = useOptions(pb.braiinsPoolStyleOptions, pb.braiinsPoolStyleToString);
    const timeFrames = useOptions(pb.braiinsPoolTimeFrameOptions, pb.braiinsPoolTimeFrameToString);

    const selectedAccount = useMemo<null | pb.Account>(() => {
        if (accountId.value == null) return null;
        return accountId.options.find(x => x.id === accountId.value) ?? null;
    }, [accountId.value, accountId.options]);
    const handleAccountChange = useCallback(
        (value: null | pb.Account): void => {
            const id = value?.id ?? null;
            if (id) accountId.onChange(id);
        },
        [accountId.onChange],
    );

    const form = (
        <Form className={css.form} style={style}>
            <WidgetSizeSelector field={widgetSize} />

            <BoundDropdown<pb.Account>
                {...accountId}
                id={$('account-id')}
                labelText={formatMessage({ defaultMessage: 'Pool Account' })}
                placeholderText="---"
                value={selectedAccount}
                onChange={handleAccountChange}
                items={accountId.options}
                itemToString={x => x?.accountName ?? 'N/A'}
                itemToElement={AccountElement}
            />

            <BoundRadioGroup
                {...sceneStyle}
                id={$('style')}
                labelText={formatMessage({ defaultMessage: 'Scene Style' })}
                items={sceneStyles}
            />

            <BoundRadioGroup
                {...timeFrame}
                id={$('time-frame')}
                labelText={formatMessage({ defaultMessage: 'Chart Time Frame' })}
                items={timeFrames}
            />

            <CheckYourScreenForPreview />

            {error ? (
                <InlineNotification
                    kind="error"
                    theme="inverse"
                    stretch
                    hideCloseButton
                    title={formatMessage({ defaultMessage: 'Error' })}
                    children={error}
                />
            ) : null}
        </Form>
    );

    const verb = isEdit ? txt.editScene : txt.addScene;

    return (
        <ModalCustom
            id={$('dialog')}
            className={css.modal}
            selectorPrimaryFocus="[role='combobox']"
            // State
            size="sm"
            open={isOpen}
            // Heading
            title={txt.braiinsPool}
            label={verb}
            // Cancel
            onClose={onClose}
            // Content
            children={form}
        />
    );
}

export function createBraiinsPoolWidgetKind(data: FormPropsToValuesRec<FormWidgetBraiinsPoolProps>): pb.WidgetKind {
    return pb.create(pb.WidgetKindSchema, {
        value: {
            case: 'braiinsPool',
            value: pb.create(pb.BraiinsPoolWidgetSchema, {
                braiinsPoolStyle: data.sceneStyle,
                timeFrame: data.timeFrame,
                accountId: data.accountId,
            }),
        },
    });
}
export function unpackBraiinsPoolWidgetKind(
    data: pb.WidgetKind,
    widgetSize: pb.WidgetSize,
): FormPropsToValuesRec<FormWidgetBraiinsPoolProps> {
    if (data.value?.case !== 'braiinsPool') throw new Error('Invalid widget kind');
    return {
        widgetSize,
        timeFrame: data.value.value.timeFrame,
        sceneStyle: data.value.value.braiinsPoolStyle,
        accountId: data.value.value.accountId,
    };
}
