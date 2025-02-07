import React from 'react';
import { Input } from '../../../../../ui/input';
import { Lock } from 'lucide-react';
import { isSecretKey } from '../../../../api_keys/utils';

interface ProviderSetupFormProps {
  requiredKeys: string[];
  configValues: { [key: string]: string };
  setConfigValues: React.Dispatch<React.SetStateAction<{ [key: string]: string }>>;
  onSubmit: (e: React.FormEvent) => void;
  provider: string;
}

/**
 * Renders the form with required input fields and the "lock" info row.
 * The submit/cancel buttons are in a separate ProviderSetupActions component.
 */
export default function ProviderSetupForm({
  requiredKeys,
  configValues,
  setConfigValues,
  onSubmit,
  provider,
}: ProviderSetupFormProps) {
  return (
    <form onSubmit={onSubmit}>
      <div className="mt-[24px] space-y-4">
        {requiredKeys.map((keyName) => (
          <div key={keyName}>
            <Input
              type={isSecretKey(keyName) ? 'password' : 'text'}
              value={configValues[keyName] || ''}
              onChange={(e) =>
                setConfigValues((prev) => ({
                  ...prev,
                  [keyName]: e.target.value,
                }))
              }
              placeholder={keyName}
              className="w-full h-14 px-4 font-regular rounded-lg border shadow-none border-gray-300 bg-white text-lg placeholder:text-gray-400 font-regular text-gray-900"
              required
            />
          </div>
        ))}
        <div className="flex text-gray-600 dark:text-gray-300">
          <Lock className="w-6 h-6" />
          <span className="text-sm font-light ml-4 mt-[2px]">
            Your configuration values will be stored securely in the keychain and used only for
            making requests to {provider}.
          </span>
        </div>
      </div>
      {/* The action buttons are not in this form; they're in ProviderSetupActions. */}
    </form>
  );
}
