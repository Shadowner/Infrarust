import mcData from 'minecraft-data';
import nmpVersion from 'minecraft-protocol/src/version.js';

const { supportedVersions } = nmpVersion;

export const INFRARUST_SUPPORTED = [
  4, 5, 47, 107, 109, 110, 335, 338, 340, 393, 477, 573, 735, 751, 754, 755, 757,
  758, 759, 760, 761, 762, 763, 764, 765, 766, 767, 768, 769, 770, 771, 772, 773,
  774, 775, 776,
];

export function allVersions() {
  return supportedVersions.map((version) => {
    const data = mcData(version);
    const protocol = data.version.version;
    return {
      version,
      label: data.version.minecraftVersion,
      protocol,
      inInfrarustTable: INFRARUST_SUPPORTED.includes(protocol),
    };
  });
}

export const TIER_B_ANCHORS = ['1.7', '1.8.8', '1.12.2', '1.16.5', '1.18.2', '1.20.1', '1.20.6', '1.21.4', '26.1'];
