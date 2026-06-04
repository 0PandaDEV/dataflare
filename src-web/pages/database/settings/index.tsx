import { IconSettings } from '@tabler/icons-react'
import { memo } from 'react'
import { useTranslation } from '../../../i18n'
import { showSettingsWindow } from '../../../tauri'
import { IconButton } from '../../../ui'
import { FeedbackSetting } from './feedback'

export const Settings = memo(() => {
    return (
        <div className='flex h-8 shrink-0 items-center justify-between gap-1 border-t border-separator px-4 py-1'>
            <FeedbackSetting />
            <SettingsEnter />
        </div>
    )
})

const SettingsEnter = () => {
    const { t } = useTranslation()

    return (
        <IconButton title={t('settings')} onClick={() => showSettingsWindow()}>
            <IconSettings size={16} stroke={1.5} />
        </IconButton>
    )
}
