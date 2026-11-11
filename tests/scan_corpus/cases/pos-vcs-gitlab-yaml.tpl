stages: [deploy]
variables:
  GITLAB_TOKEN: {{rule:gitlab-pat}}
deploy:
  script:
    - ./deploy.sh
