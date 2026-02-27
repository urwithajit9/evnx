# Improvement
- 1. Improve the add input : currently after adding a service using evnx add service postgresql creates lots of # TODO and tailing comments,
in a modified version, it would be better to have # TODO and comment as heading and value blank or default

- 2. Ouput formating.
- 3. Validate [having same variable more than once] or 'add' should check duplicate [during adding new] entry [Critical] - this function is avaliable in blueprint (it just skip and don't overright, but don't find the how many duplicates are there)
- 4. Verify the Variable name while adding using custom.

Variable name (or Enter to finish): NODE_VERSION=22
In file:

# TODO: NODE_VERSION=22=22  # <-- Fill in real value
  # (required)

- 5. validate - should also check .env .env.* in .gitignore