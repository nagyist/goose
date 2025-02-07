import React from 'react';
import { ExclamationButton, GreenCheckButton } from './actions/ActionButtons';
import {
  ConfiguredProviderTooltipMessage,
  OllamaNotConfiguredTooltipMessage,
  ProviderDescription,
} from './utils/StringUtils';

interface CardHeaderProps {
  name: string;
  description: string;
  isConfigured: boolean;
}

function CardTitle(name: string) {
  return <h3 className="text-base font-medium text-textStandard truncate mr-2">{name}</h3>;
}

function ProviderNameAndStatus(name, isConfigured) {
  const ollamaNotConfigured = !isConfigured && name === 'Ollama';
  return (
    <div className="flex items-center">
      {CardTitle(name)}

      {/* Configured state: Green check */}
      {isConfigured && <GreenCheckButton tooltip={ConfiguredProviderTooltipMessage(name)} />}

      {/* Not Configured + Ollama => Exclamation */}
      {ollamaNotConfigured && <ExclamationButton tooltip={OllamaNotConfiguredTooltipMessage()} />}
    </div>
  );
}

// Name and status icon
export default function CardHeader({ name, description, isConfigured }: CardHeaderProps) {
  return (
    <>
      <ProviderNameAndStatus name={name} isConfigured={isConfigured} />
      <ProviderDescription description={description} />
    </>
  );
}
