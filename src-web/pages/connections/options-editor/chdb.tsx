import { t } from '../../../i18n'
import { ChDbConfig } from '../../../tauri'
import { ConnectionEditorOptions } from '../connections'
import { InitialSQL, DatabasePathSelect, Item, Readonly } from '../from'
import { useOptions } from '../hooks'
import { ConnectionTab } from '../tabs'

export const ChDbConnection = ({ data, onChange }: ConnectionEditorOptions<ChDbConfig>) => {
    const { name, options, setName, setOpt } = useOptions(data, onChange)

    const general = (
        <>
            <Item label={t('name')} value={name} onChange={setName} />
            <DatabasePathSelect
                usePathAsLabel
                path={options.path}
                onChange={(path) => setOpt('path', path)}
            />
            <Item
                placeholder='default'
                label={t('database')}
                value={options.database}
                onChange={(val) => setOpt('database', val)}
            />
        </>
    )

    const security = (
        <Readonly secure={false} readonly={options.readonly} onChange={(val) => setOpt('readonly', val)} />
    )

    const initSQL = (
        <InitialSQL
            placeholder={`SET max_execution_time = 10;`}
            sql={options.initial}
            onChange={(val) => setOpt('initial', val)}
        />
    )

    return <ConnectionTab general={general} security={security} initialSQL={initSQL} alert='dev' />
}
