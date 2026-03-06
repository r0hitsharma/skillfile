class SkillfileError(Exception):
    pass


class ManifestError(SkillfileError):
    pass


class NetworkError(SkillfileError):
    pass


class InstallError(SkillfileError):
    pass
